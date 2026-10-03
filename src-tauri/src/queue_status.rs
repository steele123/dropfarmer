use crate::model::*;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

pub fn blocker(c: &Campaign, now: DateTime<Utc>) -> Option<QueueStatus> {
    let status = |state, text| Some(QueueStatus::new(state, text));
    if c.complete() {
        return status("complete", "All rewards claimed");
    }
    if !c.drops.is_empty() && !c.has_watch_rewards() {
        return status(
            "not-watchable",
            if c.drops.iter().any(|d| !d.claimed && d.required_subs > 0) {
                "Subscription required"
            } else {
                "No watch-time rewards available"
            },
        );
    }
    let (Ok(start), Ok(end)) = (
        DateTime::parse_from_rfc3339(&c.starts_at),
        DateTime::parse_from_rfc3339(&c.ends_at),
    ) else {
        return status(
            "unavailable",
            "Campaign dates unavailable — refresh campaigns",
        );
    };
    if end <= now {
        return status("ended", "Campaign ended");
    }
    if start > now {
        return status("upcoming", "Campaign has not started");
    }
    if !c.linked {
        return status("unlinked", "Link your game account");
    }
    if c.drops.is_empty() {
        return status(
            "unavailable",
            "Reward details unavailable — refresh campaigns",
        );
    }
    let pending: Vec<_> = c.drops.iter().filter(|d| !d.claimed).collect();
    let eligible: Vec<_> = pending
        .iter()
        .filter(|d| {
            d.watch_reward() && in_window(&d.starts_at, &d.ends_at, now) && d.minutes < d.required
        })
        .collect();
    if eligible.iter().any(|d| {
        d.prerequisites
            .iter()
            .all(|id| c.drops.iter().any(|p| &p.id == id && p.claimed))
    }) {
        return None;
    }
    if pending
        .iter()
        .any(|d| d.required > 0 && d.minutes >= d.required && d.claim_id.is_some())
    {
        return status("claimable", "Reward ready to claim");
    }
    if !eligible.is_empty() {
        return status(
            "prerequisite",
            "Waiting for prerequisite rewards to be claimed",
        );
    }
    if pending
        .iter()
        .any(|d| d.required > 0 && d.minutes >= d.required)
    {
        return status("confirmation", "Waiting for Twitch to confirm the reward");
    }
    if pending
        .iter()
        .any(|d| DateTime::parse_from_rfc3339(&d.starts_at).is_ok_and(|start| start > now))
    {
        return status("upcoming", "Waiting for the next reward to start");
    }
    status(
        "unavailable",
        "No rewards currently eligible for watch time",
    )
}

// Only remove known campaigns with no remaining watch rewards. Missing metadata
// must not discard a saved queue, and removing paid rewards is not completion.
pub fn remove_non_watchable(s: &mut Snapshot) -> bool {
    let before = s.queue.len();
    s.queue.retain(|id| {
        !s.campaigns
            .iter()
            .any(|c| &c.id == id && !c.drops.is_empty() && !c.complete() && !c.has_watch_rewards())
    });
    if before == s.queue.len() {
        return false;
    }
    s.sleep_plan.cancel();
    s.queue_statuses.retain(|id, _| s.queue.contains(id));
    if s.active_campaign
        .as_ref()
        .is_some_and(|id| !s.queue.contains(id))
    {
        s.active_campaign = None;
        s.channel = None;
    }
    if s.queue.is_empty() {
        s.running = false;
        s.status = "Queue empty".into();
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn campaign() -> Campaign {
        let start = "2020-01-01T00:00:00Z".to_owned();
        let end = "2100-01-01T00:00:00Z".to_owned();
        Campaign {
            id: "c".into(),
            linked: true,
            starts_at: start.clone(),
            ends_at: end.clone(),
            drops: vec![Drop {
                id: "a".into(),
                required: 30,
                starts_at: start,
                ends_at: end,
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    #[test]
    fn identifies_linking_windows_and_expiration_before_checking_streams() {
        let mut c = campaign();
        assert!(blocker(&c, Utc::now()).is_none());
        c.linked = false;
        assert_eq!(blocker(&c, Utc::now()).unwrap().state, "unlinked");
        c.starts_at = "2090-01-01T00:00:00Z".into();
        assert_eq!(blocker(&c, Utc::now()).unwrap().state, "upcoming");
        c.ends_at = "2000-01-01T00:00:00Z".into();
        assert_eq!(blocker(&c, Utc::now()).unwrap().state, "ended");
    }
    #[test]
    fn removes_paid_entries_without_losing_unknown_campaigns_or_triggering_sleep() {
        let mut paid = campaign();
        paid.id = "paid".into();
        paid.drops[0].required_subs = 1;
        assert_eq!(
            blocker(&paid, Utc::now()).unwrap().message,
            "Subscription required"
        );
        let mut empty = campaign();
        empty.id = "empty".into();
        empty.drops.clear();
        let mut s = Snapshot {
            campaigns: vec![paid, campaign(), empty],
            queue: vec!["paid".into(), "unknown".into(), "c".into(), "empty".into()],
            active_campaign: Some("paid".into()),
            running: true,
            ..Default::default()
        };
        s.sleep_plan.arm(&s.queue).unwrap();
        assert!(remove_non_watchable(&mut s));
        assert_eq!(s.queue, ["unknown", "c", "empty"]);
        assert!(s.running);
        assert!(s.active_campaign.is_none());
        assert!(!s.sleep_plan.status(std::time::Instant::now()).enabled);
        assert!(!remove_non_watchable(&mut s));
        s.queue = vec!["paid".into()];
        assert!(remove_non_watchable(&mut s));
        assert!(!s.running);
        assert_eq!(s.status, "Queue empty");
    }
    #[test]
    fn distinguishes_claim_confirmation_prerequisites_and_completion() {
        let mut c = campaign();
        c.drops[0].minutes = 30;
        assert_eq!(blocker(&c, Utc::now()).unwrap().state, "confirmation");
        c.drops[0].claim_id = Some("claim".into());
        assert_eq!(blocker(&c, Utc::now()).unwrap().state, "claimable");
        c.drops[0].minutes = 0;
        c.drops[0].claim_id = None;
        c.drops[0].prerequisites = vec!["missing".into()];
        assert_eq!(blocker(&c, Utc::now()).unwrap().state, "prerequisite");
        c.drops[0].claimed = true;
        assert_eq!(blocker(&c, Utc::now()).unwrap().state, "complete");
    }
    #[test]
    fn hides_stale_live_status_when_paused_and_does_not_invent_offline_status() {
        let mut s = Snapshot {
            campaigns: vec![campaign()],
            queue: vec!["c".into()],
            ..Default::default()
        };
        s.queue_statuses.insert(
            "c".into(),
            QueueStatus::checked("farming", "Farming on streamer", false),
        );
        assert_eq!(statuses(&s)["c"].state, "paused");
        s.running = true;
        assert_eq!(statuses(&s)["c"].state, "farming");
        s.queue_statuses.clear();
        assert_eq!(statuses(&s)["c"].state, "queued");
        s.queue_statuses.insert(
            "c".into(),
            QueueStatus::checked("offline", "No eligible channel is live", true),
        );
        assert!(statuses(&s)["c"].retry_at.is_some());
        s.needs_reconnect = true;
        assert_eq!(statuses(&s)["c"].state, "reconnect");
    }
}

pub fn statuses(s: &Snapshot) -> HashMap<String, QueueStatus> {
    let now = Utc::now();
    s.queue
        .iter()
        .map(|id| {
            let status = if s.needs_reconnect {
                QueueStatus::new("reconnect", "Reconnect Twitch to continue")
            } else if let Some(c) = s.campaigns.iter().find(|c| &c.id == id) {
                blocker(c, now).unwrap_or_else(|| {
                    if !s.running {
                        QueueStatus::new("paused", "Paused")
                    } else {
                        s.queue_statuses.get(id).cloned().unwrap_or_else(|| {
                            QueueStatus::new("queued", "Waiting for earlier campaigns")
                        })
                    }
                })
            } else {
                QueueStatus::new("unavailable", "Campaign not loaded — refresh campaigns")
            };
            (id.clone(), status)
        })
        .collect()
}
