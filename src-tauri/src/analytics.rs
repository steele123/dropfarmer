use crate::model::{Campaign, InventoryReward};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Analytics {
    pub started_at: Option<String>,
    pub campaigns: Vec<CampaignStats>,
    pub rewards: Vec<ReceivedDrop>,
    #[serde(skip_deserializing)]
    pub error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignStats {
    pub id: String,
    pub name: String,
    pub game: String,
    pub farming_ms: u64,
    pub completed: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedDrop {
    pub id: String,
    pub campaign_id: String,
    pub campaign: String,
    pub game: String,
    pub name: String,
    pub image: String,
    pub recorded_at: String,
    #[serde(default)]
    pub benefit_ids: Vec<String>,
    #[serde(default)]
    pub awarded_at: Option<String>,
    #[serde(default)]
    pub inventory_keys: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u8,
    owner: String,
    data: Analytics,
}

pub struct Ledger {
    owner: String,
    path: PathBuf,
    pub data: Analytics,
    writable: bool,
    dirty: bool,
    last_sample: Option<(Instant, Option<String>)>,
    last_save: Instant,
}

impl Ledger {
    pub fn load(root: &Path, owner: &str) -> Self {
        // Encode IDs so account names can never become filesystem paths.
        let filename: String = owner.bytes().map(|b| format!("{b:02x}")).collect();
        let path = root.join(format!("{filename}.json"));
        let loaded = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Saved>(&bytes)
                .ok()
                .filter(|saved| saved.version == 1 && saved.owner == owner)
                .map(|saved| saved.data)
                .ok_or(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Analytics::default()),
            Err(_) => Err(()),
        };
        let writable = loaded.is_ok();
        let data = loaded.unwrap_or_else(|_| Analytics {
            error: Some(
                "Saved analytics could not be read. The existing file has been kept.".into(),
            ),
            ..Default::default()
        });
        Self {
            owner: owner.into(),
            path,
            data,
            writable,
            dirty: false,
            last_sample: None,
            last_save: Instant::now(),
        }
    }

    pub fn belongs_to(&self, owner: &str) -> bool {
        self.owner == owner
    }

    // A confirmed claim remains claimed even after Twitch stops returning its self data.
    pub fn restore_claims(&self, campaigns: &mut [Campaign]) {
        for campaign in campaigns {
            for drop in &mut campaign.drops {
                if let Some(saved) = self
                    .data
                    .rewards
                    .iter()
                    .find(|r| r.campaign_id == campaign.id && r.id == drop.id)
                {
                    drop.claimed = true;
                    drop.minutes = drop.required;
                    drop.claim_id = None;
                    if drop.awarded_at.is_none() {
                        drop.awarded_at = saved.awarded_at.clone();
                    }
                }
            }
        }
    }

    pub fn observe_inventory(
        &mut self,
        campaigns: &[Campaign],
        received: &[InventoryReward],
        recorded_at: &str,
    ) -> bool {
        if !self.writable {
            return false;
        }
        let mut changed = self.observe(campaigns, recorded_at);
        for reward in received {
            let key = format!("{}:{}", reward.id, reward.awarded_at);
            if self
                .data
                .rewards
                .iter()
                .any(|r| r.inventory_keys.contains(&key))
            {
                continue;
            }
            let mapped = campaigns.iter().find_map(|c| {
                c.drops
                    .iter()
                    .find(|d| {
                        d.claimed
                            && d.benefit_ids.contains(&reward.id)
                            && chrono::DateTime::parse_from_rfc3339(&reward.awarded_at).is_ok_and(
                                |at| {
                                    crate::model::in_window(
                                        &d.starts_at,
                                        &d.ends_at,
                                        at.with_timezone(&chrono::Utc),
                                    )
                                },
                            )
                    })
                    .map(|d| (&c.id, &d.id))
            });
            if let Some((campaign, drop)) = mapped {
                if let Some(saved) = self
                    .data
                    .rewards
                    .iter_mut()
                    .find(|r| &r.campaign_id == campaign && &r.id == drop)
                {
                    saved.inventory_keys.push(key);
                    changed = true;
                    continue;
                }
            }
            // Keep received rewards even when their campaign is no longer in the catalog.
            // Unknown game/campaign metadata is not inferred from a reward's name.
            self.data.rewards.push(ReceivedDrop {
                id: format!("inventory:{}:{}", reward.id, reward.awarded_at),
                campaign_id: String::new(),
                campaign: "Twitch inventory".into(),
                game: reward.game.clone(),
                name: reward.name.clone(),
                image: reward.image.clone(),
                recorded_at: recorded_at.into(),
                benefit_ids: vec![reward.id.clone()],
                awarded_at: Some(reward.awarded_at.clone()),
                inventory_keys: vec![key],
            });
            changed = true;
        }
        self.dirty |= changed;
        changed
    }

    // Record only Twitch-confirmed claims, never submissions or local progress estimates.
    pub fn observe(&mut self, campaigns: &[Campaign], recorded_at: &str) -> bool {
        if !self.writable {
            return false;
        }
        let mut changed = false;
        for campaign in campaigns {
            if !campaign.drops.iter().any(|drop| drop.claimed) {
                continue;
            }
            let index = self.campaign(campaign);
            if campaign.complete() && !self.data.campaigns[index].completed {
                self.data.campaigns[index].completed = true;
                changed = true;
            }
            for drop in campaign.drops.iter().filter(|drop| drop.claimed) {
                // Replace inventory-only entries when campaign metadata becomes available.
                let orphan = |r: &ReceivedDrop| {
                    r.campaign_id.is_empty()
                        && r.benefit_ids.iter().any(|id| drop.benefit_ids.contains(id))
                        && r.awarded_at.as_ref().is_some_and(|at| {
                            chrono::DateTime::parse_from_rfc3339(at).is_ok_and(|at| {
                                crate::model::in_window(
                                    &drop.starts_at,
                                    &drop.ends_at,
                                    at.with_timezone(&chrono::Utc),
                                )
                            })
                        })
                };
                let first_recorded = self
                    .data
                    .rewards
                    .iter()
                    .filter(|r| orphan(r))
                    .map(|r| r.recorded_at.clone())
                    .min();
                let inventory_keys: Vec<String> = self
                    .data
                    .rewards
                    .iter()
                    .filter(|r| orphan(r))
                    .flat_map(|r| r.inventory_keys.clone())
                    .chain(drop.inventory_keys.clone())
                    .collect();
                let old_len = self.data.rewards.len();
                self.data.rewards.retain(|r| !orphan(r));
                changed |= old_len != self.data.rewards.len();
                if let Some(saved) = self
                    .data
                    .rewards
                    .iter_mut()
                    .find(|r| r.id == drop.id && r.campaign_id == campaign.id)
                {
                    for key in inventory_keys {
                        if !saved.inventory_keys.contains(&key) {
                            saved.inventory_keys.push(key);
                            changed = true;
                        }
                    }
                    if saved.benefit_ids != drop.benefit_ids && !drop.benefit_ids.is_empty() {
                        saved.benefit_ids = drop.benefit_ids.clone();
                        changed = true;
                    }
                    if saved.awarded_at.is_none() && drop.awarded_at.is_some() {
                        saved.awarded_at = drop.awarded_at.clone();
                        changed = true;
                    }
                    continue;
                }
                self.data.rewards.push(ReceivedDrop {
                    id: drop.id.clone(),
                    campaign_id: campaign.id.clone(),
                    campaign: campaign.name.clone(),
                    game: campaign.game.clone(),
                    name: drop.name.clone(),
                    image: drop.image.clone(),
                    recorded_at: first_recorded.unwrap_or_else(|| recorded_at.into()),
                    benefit_ids: drop.benefit_ids.clone(),
                    awarded_at: drop.awarded_at.clone(),
                    inventory_keys,
                });
                changed = true;
            }
        }
        self.dirty |= changed;
        changed
    }

    fn campaign(&mut self, campaign: &Campaign) -> usize {
        if let Some(index) = self.data.campaigns.iter().position(|c| c.id == campaign.id) {
            return index;
        }
        self.data.campaigns.push(CampaignStats {
            id: campaign.id.clone(),
            name: campaign.name.clone(),
            game: campaign.game.clone(),
            farming_ms: 0,
            completed: false,
        });
        self.dirty = true;
        self.data.campaigns.len() - 1
    }

    pub fn sample(&mut self, active: Option<&Campaign>, now: Instant, timestamp: &str) {
        if !self.writable {
            return;
        }
        if self.data.started_at.is_none() {
            self.data.started_at = Some(timestamp.into());
            self.dirty = true;
        }
        let current = active.map(|c| {
            self.campaign(c);
            c.id.clone()
        });
        if let Some((previous, Some(id))) = self.last_sample.take() {
            let elapsed = now.saturating_duration_since(previous);
            // Long gaps mean suspension or a stalled process; never count offline time.
            if elapsed <= Duration::from_secs(15) {
                if let Some(campaign) = self.data.campaigns.iter_mut().find(|c| c.id == id) {
                    campaign.farming_ms = campaign
                        .farming_ms
                        .saturating_add(elapsed.as_millis() as u64);
                    self.dirty |= !elapsed.is_zero();
                }
            }
        }
        self.last_sample = Some((now, current));
    }

    pub fn save(&mut self, force: bool) -> Result<(), String> {
        if !self.writable
            || !self.dirty
            || (!force && self.last_save.elapsed() < Duration::from_secs(30))
        {
            return Ok(());
        }
        let result = self.write();
        self.data.error = result.as_ref().err().cloned();
        if result.is_ok() {
            self.dirty = false;
            self.last_save = Instant::now();
        }
        result
    }

    fn write(&self) -> Result<(), String> {
        let parent = self.path.parent().ok_or("Invalid analytics folder.")?;
        std::fs::create_dir_all(parent).map_err(|_| "Could not create the analytics folder.")?;
        let bytes = serde_json::to_vec(&Saved {
            version: 1,
            owner: self.owner.clone(),
            data: self.data.clone(),
        })
        .map_err(|_| "Could not encode analytics.")?;
        let mut temp =
            tempfile::NamedTempFile::new_in(parent).map_err(|_| "Could not save analytics.")?;
        temp.write_all(&bytes)
            .map_err(|_| "Could not write analytics.")?;
        temp.as_file()
            .sync_all()
            .map_err(|_| "Could not save analytics.")?;
        temp.persist(&self.path)
            .map_err(|_| "Could not replace saved analytics.")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Drop;
    fn campaign() -> Campaign {
        Campaign {
            id: "c".into(),
            name: "Campaign".into(),
            game: "Rust".into(),
            drops: vec![Drop {
                id: "d".into(),
                name: "Reward".into(),
                required: 60,
                minutes: 60,
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    #[test]
    fn only_confirmed_claims_are_retained_once_after_restart_and_campaign_disappears() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::load(dir.path(), "a");
        let mut c = campaign();
        assert!(!ledger.observe(&[c.clone()], "first"));
        c.drops[0].claimed = true;
        assert!(ledger.observe(&[c.clone()], "first"));
        ledger.save(true).unwrap();
        let mut ledger = Ledger::load(dir.path(), "a");
        assert!(!ledger.observe(&[c.clone()], "later"));
        assert!(!ledger.observe(&[], "later"));
        assert_eq!(ledger.data.rewards.len(), 1);
        assert_eq!(ledger.data.rewards[0].recorded_at, "first");
        assert!(ledger.data.campaigns[0].completed);
        c.id = "another-campaign".into();
        assert!(ledger.observe(&[c], "later"));
        assert_eq!(ledger.data.rewards.len(), 2);
        assert!(Ledger::load(dir.path(), "b").data.rewards.is_empty());
    }
    #[test]
    fn inventory_import_enriches_without_duplicates_and_preserves_time() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::load(dir.path(), "a");
        let mut c = campaign();
        c.drops[0].starts_at = "2026-10-01T00:00:00Z".into();
        c.drops[0].ends_at = "2026-10-04T00:00:00Z".into();
        c.drops[0].benefit_ids = vec!["one".into(), "two".into()];
        let received: Vec<_> = ["one", "two"]
            .into_iter()
            .enumerate()
            .map(|(i, id)| InventoryReward {
                id: id.into(),
                name: "Reward".into(),
                image: String::new(),
                game: String::new(),
                awarded_at: format!("2026-10-02T12:0{i}:00Z"),
            })
            .collect();
        let now = Instant::now();
        ledger.sample(Some(&c), now, "start");
        ledger.sample(None, now + Duration::from_secs(10), "stop");
        assert!(ledger.observe_inventory(&[], &received, "first"));
        assert_eq!(ledger.data.rewards.len(), 2);
        assert!(!ledger.observe_inventory(&[], &received, "later"));
        c.reconcile_rewards(&received);
        assert!(c.complete());
        let single_dir = tempfile::tempdir().unwrap();
        let mut single = Ledger::load(single_dir.path(), "a");
        single.observe(&[c.clone()], "claim confirmation");
        assert!(!single.observe_inventory(&[], &received, "campaign disappeared"));
        assert_eq!(single.data.rewards.len(), 1);
        assert!(ledger.observe_inventory(&[c.clone()], &received, "later"));
        assert_eq!(
            ledger.data.rewards.len(),
            1,
            "One drop with two benefits must not appear three times"
        );
        assert_eq!(ledger.data.rewards[0].game, "Rust");
        assert_eq!(ledger.data.rewards[0].recorded_at, "first");
        assert_eq!(ledger.data.campaigns[0].farming_ms, 10000);
        assert!(ledger.data.campaigns[0].completed);
        ledger.save(true).unwrap();
        let mut ledger = Ledger::load(dir.path(), "a");
        assert!(!ledger.observe_inventory(&[], &received, "after restart"));
        assert_eq!(ledger.data.rewards.len(), 1);
        c.drops[0].claimed = false;
        c.drops[0].minutes = 0;
        ledger.restore_claims(std::slice::from_mut(&mut c));
        assert!(c.complete());
        assert_eq!(c.drops[0].minutes, 60);
        let mut other = c.clone();
        other.drops[0].claimed = false;
        Ledger::load(dir.path(), "different-account")
            .restore_claims(std::slice::from_mut(&mut other));
        assert!(!other.complete());
    }
    #[test]
    fn old_history_format_loads_and_is_enriched_without_changing_time() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::load(dir.path(), "a");
        std::fs::write(&ledger.path, serde_json::to_vec(&serde_json::json!({
            "version":1,"owner":"a","data":{"startedAt":"old","campaigns":[],"rewards":[{
                "id":"d","campaignId":"c","campaign":"Campaign","game":"Rust","name":"Reward","image":"","recordedAt":"first"
            }]}
        })).unwrap()).unwrap();
        let mut ledger = Ledger::load(dir.path(), "a");
        assert!(ledger.data.error.is_none());
        let mut c = campaign();
        ledger.restore_claims(std::slice::from_mut(&mut c));
        assert!(c.complete());
        c.drops[0].benefit_ids = vec!["benefit".into()];
        c.drops[0].awarded_at = Some("2026-10-02T12:00:00Z".into());
        assert!(ledger.observe(&[c], "later"));
        assert_eq!(ledger.data.rewards.len(), 1);
        assert_eq!(ledger.data.rewards[0].recorded_at, "first");
        assert_eq!(ledger.data.rewards[0].benefit_ids, ["benefit"]);
    }
    #[test]
    fn time_excludes_pauses_suspension_and_app_downtime() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::load(dir.path(), "a");
        let c = campaign();
        let now = Instant::now();
        ledger.sample(Some(&c), now, "start");
        ledger.sample(Some(&c), now + Duration::from_secs(5), "tick");
        ledger.sample(None, now + Duration::from_secs(7), "pause");
        ledger.sample(Some(&c), now + Duration::from_secs(12), "resume");
        ledger.sample(Some(&c), now + Duration::from_secs(3600), "after-sleep");
        ledger.sample(None, now + Duration::from_secs(3605), "stop");
        assert_eq!(ledger.data.campaigns[0].farming_ms, 12000);
        ledger.save(true).unwrap();
        let mut restarted = Ledger::load(dir.path(), "a");
        restarted.sample(None, now + Duration::from_secs(7200), "restart");
        assert_eq!(restarted.data.campaigns[0].farming_ms, 12000);
    }
    #[test]
    fn unreadable_history_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::load(dir.path(), "a");
        std::fs::write(&ledger.path, "{broken").unwrap();
        ledger = Ledger::load(dir.path(), "a");
        assert!(ledger.data.error.is_some());
        ledger.sample(Some(&campaign()), Instant::now(), "now");
        ledger.save(true).unwrap();
        assert_eq!(std::fs::read_to_string(ledger.path).unwrap(), "{broken");
    }
}
