use crate::model::{Campaign, Snapshot};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
};

const MAX_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub struct CampaignCache {
    version: u8,
    owner: String,
    login_method: String,
    pub synced_at: String,
    pub campaigns: Vec<Campaign>,
    pub notice: Option<String>,
}

impl CampaignCache {
    pub fn from_snapshot(snapshot: &Snapshot) -> Option<Self> {
        Some(Self {
            version: 1,
            owner: snapshot.account.as_ref()?.id.clone(),
            login_method: snapshot.login_method.clone()?,
            synced_at: snapshot.last_sync.clone()?,
            campaigns: snapshot.campaigns.clone(),
            notice: snapshot.campaign_notice.clone(),
        })
    }

    pub fn read(path: &Path, owner: &str, login_method: &str) -> Option<Self> {
        let file = std::fs::File::open(path).ok()?;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 > MAX_BYTES {
            return None;
        }
        let cache: Self = serde_json::from_slice(&bytes).ok()?;
        let age =
            Utc::now().signed_duration_since(DateTime::parse_from_rfc3339(&cache.synced_at).ok()?);
        if cache.version != 1
            || cache.owner != owner
            || cache.login_method != login_method
            || age.num_seconds() < -60
            || age.num_seconds() > 7 * 86400
        {
            return None;
        }
        Some(cache)
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        let parent = path.parent().ok_or("Invalid campaign cache path.")?;
        std::fs::create_dir_all(parent).map_err(|_| "Cannot create the campaign cache folder.")?;
        let bytes = serde_json::to_vec(self).map_err(|_| "Cannot encode the campaign cache.")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("Campaign cache is too large to save.".into());
        }
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| "Cannot create the campaign cache.")?;
        temporary
            .write_all(&bytes)
            .map_err(|_| "Cannot write the campaign cache.")?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| "Cannot save the campaign cache.")?;
        temporary
            .persist(path)
            .map_err(|_| "Cannot replace the campaign cache.")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Account;

    fn snapshot() -> Snapshot {
        Snapshot {
            account: Some(Account {
                id: "one".into(),
                login: "test".into(),
            }),
            login_method: Some("browser".into()),
            campaigns: vec![Campaign {
                id: "campaign".into(),
                ..Default::default()
            }],
            last_sync: Some(Utc::now().to_rfc3339()),
            ..Default::default()
        }
    }

    #[test]
    fn roundtrip_is_scoped_to_account_and_login_method() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("campaigns.json");
        CampaignCache::from_snapshot(&snapshot())
            .unwrap()
            .write(&path)
            .unwrap();
        assert_eq!(
            CampaignCache::read(&path, "one", "browser")
                .unwrap()
                .campaigns[0]
                .id,
            "campaign"
        );
        assert!(CampaignCache::read(&path, "two", "browser").is_none());
        assert!(CampaignCache::read(&path, "one", "code").is_none());
        // A successful empty catalog must replace an older populated cache.
        let mut empty = snapshot();
        empty.campaigns.clear();
        CampaignCache::from_snapshot(&empty)
            .unwrap()
            .write(&path)
            .unwrap();
        assert!(CampaignCache::read(&path, "one", "browser")
            .unwrap()
            .campaigns
            .is_empty());
    }

    #[test]
    fn failed_save_preserves_previous_campaigns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("campaigns.json");
        let mut cache = CampaignCache::from_snapshot(&snapshot()).unwrap();
        cache.write(&path).unwrap();
        cache.campaigns[0].name = "x".repeat(MAX_BYTES as usize);
        assert!(cache.write(&path).is_err());
        assert_eq!(
            CampaignCache::read(&path, "one", "browser")
                .unwrap()
                .campaigns[0]
                .id,
            "campaign"
        );
    }

    #[test]
    fn missing_corrupt_outdated_and_stale_caches_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("campaigns.json");
        assert!(CampaignCache::read(&path, "one", "browser").is_none());
        std::fs::write(&path, b"{broken").unwrap();
        assert!(CampaignCache::read(&path, "one", "browser").is_none());
        let mut cache = CampaignCache::from_snapshot(&snapshot()).unwrap();
        cache.version = 2;
        cache.write(&path).unwrap();
        assert!(CampaignCache::read(&path, "one", "browser").is_none());
        cache.version = 1;
        cache.synced_at = (Utc::now() - chrono::Duration::days(8)).to_rfc3339();
        cache.write(&path).unwrap();
        assert!(CampaignCache::read(&path, "one", "browser").is_none());
    }
}
