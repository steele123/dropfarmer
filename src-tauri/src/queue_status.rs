use crate::model::*;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

pub fn blocker(c: &Campaign, now: DateTime<Utc>) -> Option<QueueStatus> {
    let status = |state, text| Some(QueueStatus::new(state, text));
    if c.complete() {
        return status("complete", "All rewards claimed");
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
        .filter(|d| in_window(&d.starts_at, &d.ends_at, now) && d.minutes < d.required)
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
