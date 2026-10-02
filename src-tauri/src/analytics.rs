use crate::model::Campaign;
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
                if self
                    .data
                    .rewards
                    .iter()
                    .any(|r| r.id == drop.id && r.campaign_id == campaign.id)
                {
                    continue;
                }
                self.data.rewards.push(ReceivedDrop {
                    id: drop.id.clone(),
                    campaign_id: campaign.id.clone(),
                    campaign: campaign.name.clone(),
                    game: campaign.game.clone(),
                    name: drop.name.clone(),
                    image: drop.image.clone(),
                    recorded_at: recorded_at.into(),
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
