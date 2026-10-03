use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotificationSettings {
    pub rewards: bool,
    pub queue: bool,
    pub reconnect: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueStatus {
    pub state: String,
    pub message: String,
    pub checked_at: Option<String>,
    pub retry_at: Option<String>,
}
impl QueueStatus {
    pub fn new(state: &str, message: impl Into<String>) -> Self {
        Self {
            state: state.into(),
            message: message.into(),
            checked_at: None,
            retry_at: None,
        }
    }
    pub fn checked(state: &str, message: impl Into<String>, retry: bool) -> Self {
        Self {
            checked_at: Some(Utc::now().to_rfc3339()),
            retry_at: retry.then(|| (Utc::now() + chrono::Duration::seconds(60)).to_rfc3339()),
            ..Self::new(state, message)
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Drop {
    pub id: String,
    pub name: String,
    pub image: String,
    pub required: u64,
    pub minutes: u64,
    pub claimed: bool,
    pub claim_id: Option<String>,
    pub prerequisites: Vec<String>,
    pub starts_at: String,
    pub ends_at: String,
    #[serde(default)]
    pub benefit_ids: Vec<String>,
    #[serde(default)]
    pub awarded_at: Option<String>,
    #[serde(default)]
    pub inventory_keys: Vec<String>,
}
#[derive(Clone)]
pub struct InventoryReward {
    pub id: String,
    pub name: String,
    pub image: String,
    pub game: String,
    pub awarded_at: String,
}
impl InventoryReward {
    pub fn from_inventory(inventory: &Value) -> Vec<Self> {
        array(&inventory["gameEventDrops"])
            .iter()
            .filter_map(|v| {
                let id = string(v, "id");
                let awarded_at = string(v, "lastAwardedAt");
                if id.is_empty()
                    || DateTime::parse_from_rfc3339(&awarded_at).is_err()
                    || v["totalCount"].as_u64().unwrap_or(0) == 0
                {
                    return None;
                }
                Some(Self {
                    id,
                    awarded_at,
                    name: string(v, "name"),
                    image: string(v, "imageURL"),
                    game: v["game"]["displayName"]
                        .as_str()
                        .or(v["game"]["name"].as_str())
                        .unwrap_or_default()
                        .into(),
                })
            })
            .collect()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Campaign {
    pub id: String,
    pub name: String,
    pub game: String,
    pub game_id: String,
    pub image: String,
    pub linked: bool,
    pub starts_at: String,
    pub ends_at: String,
    pub channels: Vec<String>,
    pub drops: Vec<Drop>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: String,
    pub login: String,
    pub name: String,
    pub broadcast_id: String,
    pub game_id: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub login: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Log {
    pub time: String,
    pub message: String,
    pub level: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(default)]
    pub analytics: crate::analytics::Analytics,
    pub account: Option<Account>,
    pub login_method: Option<String>,
    pub campaigns: Vec<Campaign>,
    pub campaign_notice: Option<String>,
    pub campaigns_cached: bool,
    pub queue: Vec<String>,
    pub queue_statuses: HashMap<String, QueueStatus>,
    pub tray_enabled: bool,
    pub notifications: NotificationSettings,
    pub needs_reconnect: bool,
    #[serde(default)]
    pub sleep_after_queue: crate::sleep::SleepStatus,
    #[serde(skip)]
    pub sleep_plan: crate::sleep::SleepPlan,
    pub running: bool,
    pub status: String,
    pub active_campaign: Option<String>,
    pub channel: Option<Channel>,
    pub logs: Vec<Log>,
    pub last_sync: Option<String>,
    pub auto_claim: bool,
    pub error: Option<String>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            analytics: crate::analytics::Analytics::default(),
            account: None,
            login_method: None,
            campaigns: vec![],
            campaign_notice: None,
            campaigns_cached: false,
            queue: vec![],
            queue_statuses: HashMap::new(),
            tray_enabled: true,
            notifications: NotificationSettings::default(),
            needs_reconnect: false,
            sleep_after_queue: crate::sleep::SleepStatus::default(),
            sleep_plan: crate::sleep::SleepPlan::default(),
            running: false,
            status: "Twitch not connected".into(),
            active_campaign: None,
            channel: None,
            logs: vec![],
            last_sync: None,
            auto_claim: true,
            error: None,
        }
    }
}
pub fn string(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_owned()
}
pub fn array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
pub fn in_window(start: &str, end: &str, now: DateTime<Utc>) -> bool {
    match (
        DateTime::parse_from_rfc3339(start),
        DateTime::parse_from_rfc3339(end),
    ) {
        (Ok(s), Ok(e)) => s <= now && now < e,
        _ => false,
    }
}
impl Campaign {
    pub fn reconcile_rewards(&mut self, rewards: &[InventoryReward]) {
        for drop in &mut self.drops {
            // Benefit IDs identify entitlements. Names can be reused by unrelated campaigns.
            // All benefits must have been awarded during this drop's own window.
            let awards: Option<Vec<_>> = drop
                .benefit_ids
                .iter()
                .map(|id| {
                    rewards.iter().find(|r| {
                        &r.id == id
                            && DateTime::parse_from_rfc3339(&r.awarded_at).is_ok_and(|at| {
                                in_window(&drop.starts_at, &drop.ends_at, at.with_timezone(&Utc))
                            })
                    })
                })
                .collect();
            if let Some(awards) = awards.filter(|a| !a.is_empty()) {
                drop.claimed = true;
                drop.minutes = drop.required;
                drop.awarded_at = awards
                    .iter()
                    .max_by_key(|r| DateTime::parse_from_rfc3339(&r.awarded_at).ok())
                    .map(|r| r.awarded_at.clone());
                drop.claim_id = None;
                drop.inventory_keys = awards
                    .iter()
                    .map(|r| format!("{}:{}", r.id, r.awarded_at))
                    .collect();
            }
        }
    }
    pub fn can_queue(&self) -> bool {
        !self.complete()
            && DateTime::parse_from_rfc3339(&self.ends_at).is_ok_and(|end| end > Utc::now())
    }
    pub fn complete(&self) -> bool {
        !self.drops.is_empty() && self.drops.iter().all(|d| d.claimed)
    }
    pub fn next_drop(&self) -> Option<&Drop> {
        let now = Utc::now();
        if !self.linked || !in_window(&self.starts_at, &self.ends_at, now) {
            return None;
        }
        self.drops.iter().find(|d| {
            !d.claimed
                && d.required > d.minutes
                && in_window(&d.starts_at, &d.ends_at, now)
                && d.prerequisites
                    .iter()
                    .all(|p| self.drops.iter().any(|d| &d.id == p && d.claimed))
        })
    }
    pub fn from_value(v: &Value) -> Option<Self> {
        if v["game"].is_null() || string(v, "id").is_empty() {
            return None;
        }
        let starts_at = string(v, "startAt");
        let ends_at = string(v, "endAt");
        Some(Self {
            id: string(v, "id"),
            name: string(v, "name"),
            game: string(&v["game"], "displayName")
                .chars()
                .collect::<String>(),
            game_id: string(&v["game"], "id"),
            image: string(v, "imageURL"),
            linked: v["self"]["isAccountConnected"].as_bool().unwrap_or(false),
            starts_at: starts_at.clone(),
            ends_at: ends_at.clone(),
            channels: if v["allow"]["isEnabled"].as_bool().unwrap_or(true) {
                array(&v["allow"]["channels"])
                    .iter()
                    .map(|c| string(c, "name"))
                    .filter(|s| !s.is_empty())
                    .collect()
            } else {
                vec![]
            },
            drops: array(&v["timeBasedDrops"])
                .iter()
                .map(|d| {
                    let required = d["requiredMinutesWatched"].as_u64().unwrap_or(0);
                    let claimed = d["self"]["isClaimed"].as_bool().unwrap_or(false);
                    Drop {
                        id: string(d, "id"),
                        name: string(d, "name"),
                        image: array(&d["benefitEdges"])
                            .first()
                            .map(|b| string(&b["benefit"], "imageAssetURL"))
                            .unwrap_or_default(),
                        required,
                        minutes: if claimed {
                            required
                        } else {
                            d["self"]["currentMinutesWatched"]
                                .as_u64()
                                .unwrap_or(0)
                                .min(required)
                        },
                        claimed,
                        claim_id: d["self"]["dropInstanceID"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned),
                        prerequisites: array(&d["preconditionDrops"])
                            .iter()
                            .map(|p| string(p, "id"))
                            .collect(),
                        starts_at: d["startAt"].as_str().unwrap_or(&starts_at).into(),
                        ends_at: d["endAt"].as_str().unwrap_or(&ends_at).into(),
                        benefit_ids: array(&d["benefitEdges"])
                            .iter()
                            .map(|b| string(&b["benefit"], "id"))
                            .filter(|id| !id.is_empty())
                            .collect(),
                        awarded_at: None,
                        inventory_keys: vec![],
                    }
                })
                .collect(),
        })
        .map(|mut c| {
            if c.game.is_empty() {
                c.game = string(&v["game"], "name");
            }
            c
        })
    }
}
// Inventory is authoritative for progress; details fill metadata absent from inventory.
pub fn merge(primary: &Value, fallback: &Value) -> Value {
    match (primary, fallback) {
        (Value::Object(a), Value::Object(b)) => {
            let mut result = b.clone();
            for (k, v) in a {
                result.insert(k.clone(), merge(v, b.get(k).unwrap_or(&Value::Null)));
            }
            Value::Object(result)
        }
        (Value::Array(a), Value::Array(b)) if a.iter().chain(b).all(|v| v["id"].is_string()) => {
            let mut result = b.clone();
            for v in a {
                if let Some(i) = result.iter().position(|r| r["id"] == v["id"]) {
                    result[i] = merge(v, &result[i]);
                } else {
                    result.push(v.clone());
                }
            }
            Value::Array(result)
        }
        _ => primary.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn inventory_progress_wins_without_losing_details() {
        let merged = merge(
            &json!({"timeBasedDrops":[{"id":"a","self":{"currentMinutesWatched":12}}]}),
            &json!({"timeBasedDrops":[{"id":"a","requiredMinutesWatched":30,"self":{"currentMinutesWatched":0}}, {"id":"b"}]}),
        );
        assert_eq!(
            merged["timeBasedDrops"][0]["self"]["currentMinutesWatched"],
            12
        );
        assert_eq!(merged["timeBasedDrops"][0]["requiredMinutesWatched"], 30);
        assert_eq!(merged["timeBasedDrops"].as_array().unwrap().len(), 2);
    }
    #[test]
    fn prerequisites_and_linking_gate_watch() {
        let d = Drop {
            id: "a".into(),
            required: 30,
            starts_at: "2020-01-01T00:00:00Z".into(),
            ends_at: "2100-01-01T00:00:00Z".into(),
            ..Default::default()
        };
        let mut c = Campaign {
            linked: true,
            starts_at: d.starts_at.clone(),
            ends_at: d.ends_at.clone(),
            drops: vec![
                d.clone(),
                Drop {
                    id: "b".into(),
                    prerequisites: vec!["a".into()],
                    ..d
                },
            ],
            ..Default::default()
        };
        assert_eq!(c.next_drop().unwrap().id, "a");
        c.drops[0].minutes = 30;
        assert!(c.next_drop().is_none());
        c.drops[0].claimed = true;
        assert_eq!(c.next_drop().unwrap().id, "b");
        c.linked = false;
        assert!(
            c.can_queue(),
            "Account linking gates farming, not queue planning"
        );
        assert!(c.next_drop().is_none());
        c.ends_at = "2000-01-01T00:00:00Z".into();
        assert!(!c.can_queue());
    }
    #[test]
    fn malformed_or_expired_windows_cannot_earn() {
        assert!(!in_window("bad", "bad", Utc::now()));
        assert!(!in_window(
            "2000-01-01T00:00:00Z",
            "2001-01-01T00:00:00Z",
            Utc::now()
        ));
    }
}
