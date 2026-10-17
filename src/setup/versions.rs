//! Saving the config with a rolling set of earlier versions in `backups/`.

use crate::config::AppConfig;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const KEEP: usize = 20;

#[derive(Debug, Clone, Serialize)]
pub struct Version {
    pub id: String,
    /// Milliseconds since the Unix epoch
    pub saved_at: u64,
    pub pages: usize,
    pub cards: usize,
}

fn backup_dir(config_path: &Path) -> PathBuf {
    config_path.parent().unwrap_or_else(|| Path::new(".")).join("backups")
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Only plain file names we created ourselves are accepted as ids.
fn valid_id(id: &str) -> bool {
    id.starts_with("config-") && id.ends_with(".yaml") && !id.contains(['/', '\\']) && !id.contains("..")
}

/// Write `yaml` to `config_path`, first moving a differing previous version
/// into `backups/` and pruning old backups.
pub fn save(config_path: &Path, yaml: &str) -> std::io::Result<()> {
    if let Some(dir) = config_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if let Ok(previous) = std::fs::read_to_string(config_path) {
        if previous != yaml {
            let dir = backup_dir(config_path);
            std::fs::create_dir_all(&dir)?;
            let mut name = format!("config-{}.yaml", now_ms());
            // Two saves in the same millisecond: keep both.
            let mut n = 1;
            while dir.join(&name).exists() {
                name = format!("config-{}-{}.yaml", now_ms(), n);
                n += 1;
            }
            std::fs::write(dir.join(name), previous)?;
            prune(&dir);
        }
    }
    let tmp = config_path.with_extension("yaml.tmp");
    std::fs::write(&tmp, yaml)?;
    std::fs::rename(&tmp, config_path)
}

fn prune(dir: &Path) {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p| p.file_name().and_then(|n| n.to_str()).map(valid_id).unwrap_or(false));
    files.sort();
    while files.len() > KEEP {
        let _ = std::fs::remove_file(files.remove(0));
    }
}

/// Earlier versions, newest first.
pub fn list(config_path: &Path) -> Vec<Version> {
    let dir = backup_dir(config_path);
    let mut out: Vec<Version> = std::fs::read_dir(&dir)
        .map(|rd| rd.flatten().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| {
            let id = e.file_name().to_str()?.to_string();
            if !valid_id(&id) {
                return None;
            }
            let saved_at = id
                .trim_start_matches("config-")
                .trim_end_matches(".yaml")
                .split('-')
                .next()
                .and_then(|t| t.parse().ok())?;
            let config = AppConfig::load_from_file(e.path()).ok()?;
            Some(Version {
                pages: config.pages.len(),
                cards: config.pages.iter().map(|p| p.cards.len()).sum(),
                saved_at,
                id,
            })
        })
        .collect();
    out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at).then(b.id.cmp(&a.id)));
    out
}

pub fn load(config_path: &Path, id: &str) -> Result<AppConfig, String> {
    if !valid_id(id) {
        return Err("invalid version id".to_string());
    }
    AppConfig::load_from_file(backup_dir(config_path).join(id)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_previous_versions_and_prunes() {
        let dir = std::env::temp_dir().join(format!("kobo-ha-versions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.yaml");
        let yaml = |n: usize| {
            let mut c = AppConfig::default();
            c.pages.truncate(n.min(c.pages.len()));
            serde_yaml::to_string(&c).unwrap()
        };

        save(&path, &yaml(2)).unwrap();
        assert!(list(&path).is_empty(), "first save has nothing to back up");
        save(&path, &yaml(2)).unwrap();
        assert!(list(&path).is_empty(), "unchanged content is not backed up");
        save(&path, &yaml(1)).unwrap();
        let versions = list(&path);
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].pages, 2);
        assert_eq!(load(&path, &versions[0].id).unwrap().pages.len(), 2);

        for i in 0..30 {
            save(&path, &format!("version: 1\n# {}\n", i)).unwrap();
        }
        assert!(list(&path).len() <= KEEP);
        assert!(load(&path, "../config.yaml").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
