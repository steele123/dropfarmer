use crate::model::{in_window, Snapshot};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CHECK_SECONDS: u64 = 15 * 60;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FollowedGame {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AutoFarm {
    pub enabled: bool,
    pub games: Vec<FollowedGame>,
    // Keep handled drop IDs even when Twitch temporarily omits a campaign.
    pub handled: BTreeMap<String, BTreeSet<String>>,
}

/// Only fresh, authenticated discovery may enqueue work. A handled reward is
/// never re-added after manual removal; a new drop in the same campaign is.
pub fn enqueue(s: &mut Snapshot, now: DateTime<Utc>) -> Vec<String> {
    if !s.auto_farm.enabled || s.account.is_none() || s.needs_reconnect || s.campaigns_cached {
        return vec![];
    }
    let mut added = Vec::new();
    for c in &s.campaigns {
        if !s.auto_farm.games.iter().any(|g| g.id == c.game_id)
            || !c.linked
            || !in_window(&c.starts_at, &c.ends_at, now)
        {
            continue;
        }
        let eligible: BTreeSet<String> = c
            .drops
            .iter()
            .filter(|d| {
                !d.id.is_empty()
                    && !d.claimed
                    && d.watch_reward()
                    && in_window(&d.starts_at, &d.ends_at, now)
                    && d.prerequisites
                        .iter()
                        .all(|id| c.drops.iter().any(|p| &p.id == id && p.claimed))
            })
            .map(|d| d.id.clone())
            .collect();
        if eligible.is_empty() {
            continue;
        }
        let handled = s.auto_farm.handled.entry(c.id.clone()).or_default();
        if eligible.is_subset(handled) {
            continue;
        }
        if !s.queue.contains(&c.id) {
            if s.queue.len() >= 200 {
                continue;
            }
            s.queue.push(c.id.clone());
        }
        added.push(c.name.clone());
        // Include later rewards in this campaign so unlocking a prerequisite
        // cannot undo a manual removal on the next check.
        handled.extend(
            c.drops
                .iter()
                .filter(|d| !d.claimed && d.watch_reward())
                .map(|d| d.id.clone()),
        );
    }
    if !added.is_empty() {
        s.sleep_plan.cancel();
        if !s.running {
            s.running = true;
            s.status = "Finding an eligible stream…".into();
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Account, Campaign};
    use serde_json::json;

    fn state() -> Snapshot {
        let campaign = Campaign::from_value(&json!({
            "id":"campaign", "name":"New drops", "game":{"id":"game", "displayName":"Rust"},
            "self":{"isAccountConnected":true}, "startAt":"2020-01-01T00:00:00Z", "endAt":"2100-01-01T00:00:00Z",
            "timeBasedDrops":[{"id":"drop", "requiredMinutesWatched":30}]
        })).unwrap();
        Snapshot {
            account: Some(Account {
                id: "owner".into(),
                login: "viewer".into(),
            }),
            campaigns: vec![campaign],
            auto_farm: AutoFarm {
                enabled: true,
                games: vec![FollowedGame {
                    id: "game".into(),
                    name: "Rust".into(),
                }],
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn appends_once_and_detects_new_drops_in_existing_campaigns() {
        let mut s = state();
        s.queue.push("manual".into());
        assert_eq!(enqueue(&mut s, Utc::now()), ["New drops"]);
        assert_eq!(s.queue, ["manual", "campaign"]);
        assert!(s.running);
        s.queue.clear();
        s.running = false;
        s.auto_farm = serde_json::from_str(&serde_json::to_string(&s.auto_farm).unwrap()).unwrap();
        assert!(enqueue(&mut s, Utc::now()).is_empty());
        assert!(!s.running);
        let mut new_drop = s.campaigns[0].drops[0].clone();
        new_drop.id = "new".into();
        s.campaigns[0].drops.push(new_drop);
        assert_eq!(enqueue(&mut s, Utc::now()), ["New drops"]);
    }

    #[test]
    fn rejects_paid_claimed_unlinked_future_expired_and_cached_rewards() {
        for kind in 0..9 {
            let mut s = state();
            match kind {
                0 => s.campaigns[0].drops[0].required_subs = 1,
                1 => s.campaigns[0].drops[0].claimed = true,
                2 => s.campaigns[0].linked = false,
                3 => s.campaigns[0].drops[0].starts_at = "2099-01-01T00:00:00Z".into(),
                4 => s.campaigns[0].ends_at = "2021-01-01T00:00:00Z".into(),
                5 => s.campaigns_cached = true,
                6 => s.needs_reconnect = true,
                7 => s.auto_farm.enabled = false,
                _ => s.campaigns[0].drops[0].prerequisites.push("paid".into()),
            }
            assert!(enqueue(&mut s, Utc::now()).is_empty(), "case {kind}");
            assert!(s.auto_farm.handled.is_empty());
        }
    }

    #[test]
    fn future_or_unlinked_rewards_can_be_picked_up_later() {
        let mut s = state();
        s.campaigns[0].linked = false;
        enqueue(&mut s, Utc::now());
        s.campaigns[0].linked = true;
        s.campaigns[0].drops[0].starts_at = "2099-01-01T00:00:00Z".into();
        enqueue(&mut s, Utc::now());
        assert!(s.queue.is_empty());
        assert_eq!(
            enqueue(&mut s, "2099-02-01T00:00:00Z".parse().unwrap()).len(),
            1
        );
    }

    #[test]
    fn respects_queue_limit_and_does_not_forget_missing_campaigns() {
        let mut s = state();
        s.queue = (0..200).map(|i| i.to_string()).collect();
        assert!(enqueue(&mut s, Utc::now()).is_empty());
        s.queue.pop();
        assert_eq!(enqueue(&mut s, Utc::now()).len(), 1);
        s.queue.clear();
        let campaigns = std::mem::take(&mut s.campaigns);
        enqueue(&mut s, Utc::now());
        s.campaigns = campaigns;
        assert!(enqueue(&mut s, Utc::now()).is_empty());
    }

    #[test]
    fn unlocking_existing_rewards_does_not_undo_manual_removal() {
        let mut s = state();
        let mut later = s.campaigns[0].drops[0].clone();
        later.id = "later".into();
        later.prerequisites = vec!["drop".into()];
        s.campaigns[0].drops.push(later);
        enqueue(&mut s, Utc::now());
        s.queue.clear();
        s.campaigns[0].drops[0].claimed = true;
        assert!(enqueue(&mut s, Utc::now()).is_empty());
        assert!(s.queue.is_empty());
    }

    #[test]
    fn new_campaign_is_detected_and_existing_queue_order_is_preserved() {
        let mut s = state();
        s.queue = vec!["manual".into(), "campaign".into()];
        s.sleep_plan.arm(&s.queue).unwrap();
        assert_eq!(enqueue(&mut s, Utc::now()).len(), 1);
        assert_eq!(s.queue, ["manual", "campaign"]);
        assert!(!s.sleep_plan.status(std::time::Instant::now()).enabled);
        assert!(enqueue(&mut s, Utc::now()).is_empty());
        let mut new = s.campaigns[0].clone();
        new.id = "new-campaign".into();
        s.campaigns.push(new);
        assert_eq!(enqueue(&mut s, Utc::now()).len(), 1);
        assert_eq!(s.queue, ["manual", "campaign", "new-campaign"]);
    }

    #[test]
    fn requires_a_session_and_the_followed_game_id() {
        let mut s = state();
        s.account = None;
        assert!(enqueue(&mut s, Utc::now()).is_empty());
        let mut s = state();
        s.campaigns[0].game_id = "different-game-with-same-name".into();
        assert!(enqueue(&mut s, Utc::now()).is_empty());
        assert!(s.queue.is_empty());
    }
}
