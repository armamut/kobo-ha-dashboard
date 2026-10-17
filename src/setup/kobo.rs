//! Finding a USB-connected Kobo and installing the dashboard onto it.

use crate::config::AppConfig;
use crate::models;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const LAUNCH_SH: &str = include_str!("../../deploy/launch.sh");
pub const DEFAULT_MENU_NAME: &str = "Home Dashboard";

#[derive(Debug, Clone, Serialize)]
pub struct KoboDevice {
    pub path: String,
    pub firmware: Option<String>,
    pub model_id: Option<String>,
    /// Key into `models::MODELS`, when the model is known
    pub model: Option<String>,
    pub model_name: Option<String>,
    /// "installed", "pending" (waiting in .kobo/KoboRoot.tgz) or "missing"
    pub nickelmenu: &'static str,
    /// NickelMenu supports firmware 4.6 up to (not including) 5.x
    pub firmware_supported: bool,
    pub installed: bool,
    pub has_config: bool,
}

fn candidate_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut push_children = |dir: &Path| {
        if let Ok(rd) = std::fs::read_dir(dir) {
            roots.extend(rd.flatten().map(|e| e.path()));
        }
    };
    if cfg!(target_os = "macos") {
        push_children(Path::new("/Volumes"));
    } else if cfg!(windows) {
        for letter in b'D'..=b'Z' {
            roots.push(PathBuf::from(format!("{}:\\", letter as char)));
        }
    } else {
        let user = std::env::var("USER").unwrap_or_default();
        for base in [format!("/media/{}", user), format!("/run/media/{}", user), "/media".into(), "/mnt".into()] {
            push_children(Path::new(&base));
        }
    }
    roots
}

pub fn detect() -> Vec<KoboDevice> {
    candidate_roots()
        .into_iter()
        .filter(|root| root.join(".kobo").is_dir())
        .map(|root| {
            // .kobo/version: serial,kernel,firmware,...,product UUID
            let version = std::fs::read_to_string(root.join(".kobo/version")).unwrap_or_default();
            let fields: Vec<&str> = version.trim().split(',').collect();
            let model_id = fields.last().and_then(|s| s.rsplit('-').next()).map(|s| s.trim_start_matches('0').to_string());
            let model = model_id.as_deref().and_then(models::by_model_id);
            KoboDevice {
                firmware: fields.get(2).map(|s| s.to_string()),
                model: model.map(|m| m.key.to_string()),
                model_name: model.map(|m| m.name.to_string()),
                model_id,
                nickelmenu: nickelmenu_status(&root),
                firmware_supported: fields.get(2).map(|f| firmware_supported(f)).unwrap_or(true),
                installed: root.join(".kobo-ha/kobo-ha-dashboard").is_file(),
                has_config: root.join(".kobo-ha/config.yaml").is_file(),
                path: root.to_string_lossy().into_owned(),
            }
        })
        .collect()
}

/// NickelMenu's package installs `.adds/nm/doc`, so that file means it has
/// been installed (our own menu entry also lives in `.adds/nm`, so the folder
/// alone proves nothing).
fn nickelmenu_status(root: &Path) -> &'static str {
    if root.join(".adds/nm/doc").is_file() && !root.join(".adds/nm/uninstall").exists() {
        "installed"
    } else if root.join(".kobo/KoboRoot.tgz").is_file() {
        "pending"
    } else {
        "missing"
    }
}

fn firmware_supported(version: &str) -> bool {
    let mut parts = version.split('.').filter_map(|p| p.parse::<u32>().ok());
    match (parts.next(), parts.next()) {
        (Some(4), Some(minor)) => minor >= 6,
        (Some(major), _) => major < 5 && major >= 4,
        _ => true,
    }
}

/// Put NickelMenu's KoboRoot.tgz where the Kobo picks up updates. It is
/// installed by the Kobo itself on the next restart.
pub fn install_nickelmenu(root: &Path, package: &[u8]) -> Result<(), String> {
    if !root.join(".kobo").is_dir() {
        return Err(format!("{} does not look like a Kobo (no .kobo folder)", root.display()));
    }
    let target = root.join(".kobo/KoboRoot.tgz");
    if let Ok(existing) = std::fs::read(&target) {
        if existing != package {
            // Don't overwrite some other pending update (e.g. another add-on).
            return Err("other_package_pending".to_string());
        }
    }
    let _ = std::fs::remove_file(root.join(".adds/nm/uninstall"));
    std::fs::write(&target, package).map_err(|e| e.to_string())?;
    remove_apple_double(&root.join(".kobo"));
    Ok(())
}

/// Locate the ARM (Kobo) build of the dashboard to copy onto the device.
pub fn find_kobo_binary(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return p.is_file().then(|| p.to_path_buf());
    }
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    let mut candidates = Vec::new();
    if let Some(dir) = &exe_dir {
        candidates.push(dir.join("kobo").join("kobo-ha-dashboard"));
        candidates.push(dir.join("kobo-ha-dashboard-kobo"));
    }
    candidates.push(PathBuf::from("kobo/kobo-ha-dashboard"));
    candidates.push(PathBuf::from("target/armv7-unknown-linux-musleabihf/release/kobo-ha-dashboard"));
    candidates.push(PathBuf::from("deploy/kobo-ha-dashboard"));
    candidates.into_iter().find(|p| p.is_file())
}

pub fn menu_file(name: &str) -> String {
    let name = name.replace(':', " ").trim().to_string();
    let name = if name.is_empty() { DEFAULT_MENU_NAME.to_string() } else { name };
    format!(
        "# NickelMenu entry for Kobo HA Dashboard (created by the setup tool)\n\
         menu_item :main :{0} :cmd_spawn :quiet :/bin/sh /mnt/onboard/.kobo-ha/launch.sh\n\
         menu_item :reader :{0} :cmd_spawn :quiet :/bin/sh /mnt/onboard/.kobo-ha/launch.sh\n",
        name
    )
}

/// Read the current menu name back from a device, if installed.
pub fn read_menu_name(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join(".adds/nm/kobo-ha")).ok()?;
    text.lines()
        .find(|l| l.starts_with("menu_item"))
        .and_then(|l| l.split(':').nth(2))
        .map(|s| s.trim().to_string())
}

fn remove_apple_double(dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.file_name().map(|n| n.to_string_lossy().starts_with("._")).unwrap_or(false) {
                let _ = std::fs::remove_file(&p);
            } else if p.is_dir() {
                remove_apple_double(&p);
            }
        }
    }
}

/// A translatable log/message line: `key` under `install.log.` / `report.`
#[derive(Debug, Clone, Serialize)]
pub struct Msg {
    pub key: String,
    pub args: BTreeMap<String, String>,
}

impl Msg {
    pub fn new(key: &str) -> Self {
        Self { key: key.to_string(), args: BTreeMap::new() }
    }

    pub fn arg(mut self, name: &str, value: impl Into<String>) -> Self {
        self.args.insert(name.to_string(), value.into());
        self
    }
}

/// Copy binary, launcher, config, menu entry and fonts. Returns a log.
pub fn install(
    root: &Path,
    config: &AppConfig,
    menu_name: &str,
    kobo_binary: &Path,
    fonts: &[PathBuf],
) -> Result<Vec<Msg>, String> {
    let mut log = Vec::new();
    let err = |what: &str, e: std::io::Error| format!("{}: {}", what, e);
    if !root.join(".kobo").is_dir() {
        return Err(format!("{} does not look like a Kobo (no .kobo folder)", root.display()));
    }

    let app = root.join(".kobo-ha");
    std::fs::create_dir_all(&app).map_err(|e| err("Creating .kobo-ha", e))?;

    std::fs::copy(kobo_binary, app.join("kobo-ha-dashboard")).map_err(|e| err("Copying the dashboard program", e))?;
    log.push(Msg::new("binary"));

    std::fs::write(app.join("launch.sh"), LAUNCH_SH).map_err(|e| err("Writing launch.sh", e))?;
    log.push(Msg::new("launcher"));

    let cfg_path = app.join("config.yaml");
    if cfg_path.exists() {
        std::fs::copy(&cfg_path, app.join("config.yaml.bak")).map_err(|e| err("Backing up config.yaml", e))?;
        log.push(Msg::new("backup"));
    }
    let yaml = serde_yaml::to_string(config).map_err(|e| e.to_string())?;
    std::fs::write(&cfg_path, yaml).map_err(|e| err("Writing config.yaml", e))?;
    log.push(Msg::new("config"));

    if !fonts.is_empty() {
        let font_dir = app.join("fonts");
        std::fs::create_dir_all(&font_dir).map_err(|e| err("Creating fonts folder", e))?;
        for f in fonts {
            if let Some(name) = f.file_name() {
                std::fs::copy(f, font_dir.join(name)).map_err(|e| err("Copying font", e))?;
                log.push(Msg::new("font").arg("name", name.to_string_lossy()));
            }
        }
    }

    let nm = root.join(".adds/nm");
    std::fs::create_dir_all(&nm).map_err(|e| err("Creating .adds/nm", e))?;
    std::fs::write(nm.join("kobo-ha"), menu_file(menu_name)).map_err(|e| err("Writing NickelMenu entry", e))?;
    log.push(Msg::new("menu").arg("name", menu_name.trim()));

    // macOS leaves ._ files on FAT volumes; NickelMenu would try to parse them.
    remove_apple_double(&app);
    remove_apple_double(&nm);

    log.push(Msg::new("done"));
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_writes_everything_and_backs_up() {
        let root = std::env::temp_dir().join(format!("kobo-ha-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".kobo")).unwrap();
        std::fs::create_dir_all(root.join(".kobo-ha")).unwrap();
        std::fs::write(root.join(".kobo-ha/config.yaml"), "old").unwrap();
        std::fs::create_dir_all(root.join(".adds/nm")).unwrap();
        std::fs::write(root.join(".adds/nm/._kobo-ha"), "junk").unwrap();
        let bin = root.join("fake-bin");
        std::fs::write(&bin, "ELF").unwrap();

        let log = install(&root, &AppConfig::default(), "My: Home", &bin, &[]).unwrap();
        assert_eq!(log.last().unwrap().key, "done");
        assert_eq!(std::fs::read_to_string(root.join(".kobo-ha/config.yaml.bak")).unwrap(), "old");
        assert!(root.join(".kobo-ha/kobo-ha-dashboard").is_file());
        assert!(!root.join(".adds/nm/._kobo-ha").exists());
        assert_eq!(read_menu_name(&root).as_deref(), Some("My  Home"));

        // Our menu entry created .adds/nm, but NickelMenu itself is not installed
        assert_eq!(nickelmenu_status(&root), "missing");
        install_nickelmenu(&root, b"pkg").unwrap();
        assert_eq!(nickelmenu_status(&root), "pending");
        assert_eq!(install_nickelmenu(&root, b"other").unwrap_err(), "other_package_pending");
        std::fs::write(root.join(".adds/nm/doc"), "doc").unwrap();
        assert_eq!(nickelmenu_status(&root), "installed");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn firmware_support() {
        assert!(firmware_supported("4.46.23836"));
        assert!(firmware_supported("4.6.9960"));
        assert!(!firmware_supported("4.5.9587"));
        assert!(!firmware_supported("5.2.1"));
    }
}
