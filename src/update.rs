//! New versions: once a day the latest release on GitHub is read, and if it
//! is newer than this program (and not skipped) the setup tool and the
//! dashboard say so, with the release notes. Installing stays with the
//! setup tool. What was seen and skipped is kept in `update-state.json`
//! next to the config.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where the releases are published
pub const REPO: &str = "armamut/kobo-ha-dashboard";
/// This program's version
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// How often GitHub is asked
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Release {
    /// "0.4.1", without the "v"
    pub version: String,
    /// The release title, e.g. "v0.4.1: Grid designer"
    pub title: String,
    /// Release notes as written on GitHub (Markdown)
    pub notes: String,
    /// The release page
    pub url: String,
    /// (file name, download link) of each package
    pub assets: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateState {
    /// When GitHub was last asked (Unix seconds), whatever the answer
    #[serde(default)]
    pub last_check: Option<i64>,
    /// The latest release seen
    #[serde(default)]
    pub latest: Option<Release>,
    /// A version the user chose to skip: it is not offered again
    #[serde(default)]
    pub skipped: Option<String>,
}

pub fn state_path(config_dir: &Path) -> PathBuf {
    config_dir.join("update-state.json")
}

/// The dashboard's config folder, where its state file lives
static STATE_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

pub fn set_state_dir(dir: Option<PathBuf>) {
    if let Ok(mut d) = STATE_DIR.lock() {
        *d = dir;
    }
}

fn state_file() -> Option<PathBuf> {
    STATE_DIR.lock().ok().and_then(|d| d.as_deref().map(state_path))
}

/// What the dashboard offers: (version, release notes as plain text)
pub fn current_offer() -> Option<(String, String)> {
    let state = UpdateState::load(&state_file()?);
    state.offer(CURRENT).map(|r| (r.version.clone(), plain_notes(&r.notes)))
}

/// Don't offer `version` again on this device
pub fn skip(version: &str) {
    if let Some(path) = state_file() {
        let mut state = UpdateState::load(&path);
        state.skipped = Some(version.to_string());
        state.save(&path);
    }
}

/// The dashboard's own check, see [`check`]
pub async fn check_here() -> bool {
    match state_file() {
        Some(path) => check(&path).await,
        None => false,
    }
}

impl UpdateState {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    /// Time to ask GitHub again
    pub fn due(&self, now: i64) -> bool {
        self.last_check.is_none_or(|t| now - t >= CHECK_EVERY.as_secs() as i64 || now < t)
    }

    /// The release to tell about: newer than `current` and not skipped
    pub fn offer(&self, current: &str) -> Option<&Release> {
        self.latest
            .as_ref()
            .filter(|r| is_newer(&r.version, current) && self.skipped.as_deref() != Some(r.version.as_str()))
    }
}

/// "0.4.1" > "0.3.21"; a version that does not parse is never newer
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse(candidate), parse(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

fn parse(v: &str) -> Option<(u32, u32, u32)> {
    let v = v.trim().trim_start_matches('v');
    // A pre-release ("0.5.0-beta.1") is not offered
    if v.contains('-') {
        return None;
    }
    let mut parts = v.split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next().flatten().unwrap_or(0), parts.next().flatten().unwrap_or(0)))
}

/// The latest release on GitHub. Only a plain client name goes with the
/// request: nothing about the user or their setup.
pub async fn fetch_latest() -> Result<Release, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(format!("kobo-ha-dashboard/{}", CURRENT))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(format!("https://api.github.com/repos/{}/releases/latest", REPO))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    release_from_json(&v).ok_or_else(|| "unexpected answer from GitHub".to_string())
}

fn release_from_json(v: &serde_json::Value) -> Option<Release> {
    if v.get("draft").and_then(|d| d.as_bool()) == Some(true) || v.get("prerelease").and_then(|d| d.as_bool()) == Some(true) {
        return None;
    }
    let tag = v.get("tag_name")?.as_str()?;
    Some(Release {
        version: tag.trim_start_matches('v').to_string(),
        title: v.get("name").and_then(|n| n.as_str()).unwrap_or(tag).to_string(),
        notes: v.get("body").and_then(|b| b.as_str()).unwrap_or("").to_string(),
        url: v.get("html_url").and_then(|u| u.as_str()).unwrap_or("").to_string(),
        assets: v
            .get("assets")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| Some((x.get("name")?.as_str()?.to_string(), x.get("browser_download_url")?.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Ask GitHub if it is time, keep the answer, and say whether anything new
/// came in. A failed request also counts as a check, but only for an hour,
/// so a Kobo without Wi-Fi does not try on every refresh.
pub async fn check(path: &Path) -> bool {
    let mut state = UpdateState::load(path);
    let now = chrono::Utc::now().timestamp();
    if !state.due(now) {
        return false;
    }
    match fetch_latest().await {
        Ok(release) => {
            let new = state.latest.as_ref() != Some(&release);
            state.latest = Some(release);
            state.last_check = Some(now);
            state.save(path);
            new
        }
        Err(e) => {
            tracing::info!("Could not check for a new version: {}", e);
            state.last_check = Some(now - CHECK_EVERY.as_secs() as i64 + 3600);
            state.save(path);
            false
        }
    }
}

/// The package for this computer, from a release's files
pub fn asset_for_this_system(release: &Release) -> Option<&(String, String)> {
    let want = if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "macos-apple-silicon"
    } else if cfg!(target_os = "macos") {
        "macos-intel"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    release.assets.iter().find(|(name, _)| name.contains(want))
}

/// Release notes as plain text for the e-ink screen: headings, emphasis,
/// links and code marks taken out, list items kept as "• ".
pub fn plain_notes(markdown: &str) -> String {
    let mut out = Vec::new();
    for line in markdown.lines() {
        let line = line.trim_end();
        let line = line.trim_start_matches('#').trim_start();
        let line = if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            format!("• {}", rest)
        } else {
            line.to_string()
        };
        // [text](link) -> text
        let mut text = String::new();
        let mut rest = line.as_str();
        while let Some(open) = rest.find('[') {
            match (rest[open..].find("]("), rest[open..].find(')')) {
                (Some(mid), Some(close)) if mid < close => {
                    text.push_str(&rest[..open]);
                    text.push_str(&rest[open + 1..open + mid]);
                    rest = &rest[open + close + 1..];
                }
                _ => break,
            }
        }
        text.push_str(rest);
        out.push(text.replace("**", "").replace('`', ""));
    }
    out.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("0.4.0", "0.3.21"));
        assert!(is_newer("v0.3.22", "0.3.21"));
        assert!(!is_newer("0.3.21", "0.3.21"));
        assert!(!is_newer("0.3.9", "0.3.21"));
        assert!(!is_newer("0.5.0-beta.1", "0.4.0"));
        assert!(!is_newer("nonsense", "0.4.0"));
    }

    #[test]
    fn a_skipped_or_older_release_is_not_offered() {
        let r = |v: &str| Release {
            version: v.into(),
            title: v.into(),
            notes: String::new(),
            url: String::new(),
            assets: Vec::new(),
        };
        let mut s = UpdateState {
            latest: Some(r("0.4.1")),
            ..Default::default()
        };
        assert_eq!(s.offer("0.4.0").map(|r| r.version.as_str()), Some("0.4.1"));
        assert!(s.offer("0.4.1").is_none());
        s.skipped = Some("0.4.1".into());
        assert!(s.offer("0.4.0").is_none());
        // A newer one than the skipped one is offered again
        s.latest = Some(r("0.4.2"));
        assert!(s.offer("0.4.0").is_some());
    }

    #[test]
    fn a_check_is_due_once_a_day() {
        let mut s = UpdateState::default();
        assert!(s.due(1_000_000));
        s.last_check = Some(1_000_000);
        assert!(!s.due(1_000_000 + 3600));
        assert!(s.due(1_000_000 + 25 * 3600));
        // A clock that went back (a Kobo without time yet) asks again
        assert!(s.due(999_000));
    }

    #[test]
    fn a_github_release_is_read() {
        let v = serde_json::json!({
            "tag_name": "v0.4.0", "name": "v0.4.0: Grid designer", "body": "## New\n- **Grid** designer",
            "html_url": "https://github.com/x/y/releases/tag/v0.4.0", "draft": false, "prerelease": false,
            "assets": [{ "name": "kobo-ha-dashboard-v0.4.0-macos-apple-silicon.zip", "browser_download_url": "https://dl/mac" }]
        });
        let r = release_from_json(&v).unwrap();
        assert_eq!(r.version, "0.4.0");
        assert_eq!(r.assets.len(), 1);
        assert!(release_from_json(&serde_json::json!({ "tag_name": "v1", "prerelease": true })).is_none());
    }

    #[test]
    fn notes_read_as_plain_text() {
        let md = "## What's new\n- **Grid** designer, see [the docs](https://x)\n* `cover` card";
        assert_eq!(plain_notes(md), "What's new\n• Grid designer, see the docs\n• cover card");
    }
}
