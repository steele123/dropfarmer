use crate::model::Campaign;

pub fn requires_reconnect(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    [
        "browser session expired",
        "browser session data is invalid",
        "browser session proof",
        "saved authorization",
        "saved twitch session is invalid",
        "http 401",
        "http 403",
        "denied access for this session",
    ]
    .iter()
    .any(|reason| error.contains(reason))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Drop;
    #[test]
    fn only_confirmed_transitions_trigger_claim_notifications() {
        let before = vec![Campaign {
            id: "c".into(),
            game: "Rust".into(),
            drops: vec![Drop {
                id: "d".into(),
                name: "Rifle".into(),
                minutes: 120,
                required: 120,
                ..Default::default()
            }],
            ..Default::default()
        }];
        assert!(newly_claimed(&before, &before).is_empty());
        let mut after = before.clone();
        after[0].drops[0].claimed = true;
        assert_eq!(
            newly_claimed(&before, &after),
            [("c:d".into(), "Rifle · Rust".into())]
        );
        assert!(newly_claimed(&after, &after).is_empty());
        assert!(
            newly_claimed(&[], &after).is_empty(),
            "Initial inventory must not announce old claims"
        );
    }
    #[test]
    fn transient_errors_do_not_request_a_new_login() {
        for error in [
            "Cannot reach Twitch. Check your connection.",
            "Twitch returned HTTP 500 Internal Server Error. Please retry or reconnect Twitch.",
            "Twitch returned HTTP 429 Too Many Requests. Please retry or reconnect Twitch.",
            "Twitch no longer recognizes this query.",
        ] {
            assert!(!requires_reconnect(error));
        }
        for error in [
            "Browser session expired. Connect Twitch using browser sign-in again.",
            "Twitch rejected the saved authorization. Sign in again.",
            "Twitch returned HTTP 401 Unauthorized. Please retry or reconnect Twitch.",
        ] {
            assert!(requires_reconnect(error));
        }
    }
}

pub fn newly_claimed(before: &[Campaign], after: &[Campaign]) -> Vec<(String, String)> {
    after
        .iter()
        .flat_map(|c| {
            let old = before.iter().find(|old| old.id == c.id);
            c.drops.iter().filter_map(move |d| {
                (d.claimed
                    && old.is_some_and(|old| {
                        old.drops
                            .iter()
                            .any(|previous| previous.id == d.id && !previous.claimed)
                    }))
                .then(|| {
                    (
                        format!("{}:{}", c.id, d.id),
                        format!("{} · {}", d.name, c.game),
                    )
                })
            })
        })
        .collect()
}
