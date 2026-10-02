use std::{io::Write, path::Path};

// A fixed destination and generated filename keep this command scoped to CSV exports.
pub fn save_csv(folder: &Path, csv: &str) -> Result<String, String> {
    if csv.is_empty() || csv.len() > 20 * 1024 * 1024 {
        return Err("The export is empty or exceeds 20 MB.".into());
    }
    std::fs::create_dir_all(folder).map_err(|_| "Could not access your Downloads folder.")?;
    let mut file =
        tempfile::NamedTempFile::new_in(folder).map_err(|_| "Could not create the CSV export.")?;
    file.write_all(csv.as_bytes())
        .map_err(|_| "Could not write the CSV export.")?;
    file.as_file()
        .sync_all()
        .map_err(|_| "Could not save the CSV export.")?;
    let stamp = chrono::Utc::now().format("%Y-%m-%d_%H%M%S");
    for attempt in 0..1000 {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let path = folder.join(format!("dropfarmer-rewards-{stamp}{suffix}.csv"));
        match file.persist_noclobber(&path) {
            Ok(_) => return Ok(path.to_string_lossy().into_owned()),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                file = error.file
            }
            Err(_) => return Err("Could not save the CSV export in Downloads.".into()),
        }
    }
    Err("Could not find a free filename for the export.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_exports_preserve_existing_files_and_unicode() {
        let dir = tempfile::tempdir().unwrap();
        let csv = "\u{feff}\"Reward\"\r\n\"Épée\"\r\n";
        let first = save_csv(dir.path(), csv).unwrap();
        let second = save_csv(dir.path(), "second export").unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(first).unwrap(), csv);
        assert_eq!(std::fs::read_to_string(second).unwrap(), "second export");
        assert!(save_csv(dir.path(), "").is_err());
    }
}
