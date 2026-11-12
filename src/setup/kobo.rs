//! Finding a USB-connected Kobo and installing the dashboard onto it.

use crate::config::AppConfig;
use crate::models;
use crate::paths;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const LAUNCH_SH: &str = include_str!("../../deploy/launch.sh");
pub const WIFI_SH: &str = include_str!("../../deploy/wifi.sh");
pub const DEFAULT_MENU_NAME: &str = "Home Dashboard";

#[derive(Debug, Clone, Serialize)]
pub struct KoboDevice {
    pub path: String,
    pub firmware: Option<String>,
    pub model_id: Option<String>,
    /// Key into `models::MODELS`, when the model is known
    pub model: Option<String>,
    pub model_name: Option<String>,
    /// "installed", "pending" (a package waiting in .kobo), "check" (the
    /// Kobo software changed since the dashboard last started from the menu:
    /// updates remove NickelMenu) or "missing"
    pub nickelmenu: &'static str,
    /// The NickelMenu release for this firmware, and whether it is the fork
    pub nickelmenu_version: Option<&'static str>,
    pub nickelmenu_fork: bool,
    /// There is a NickelMenu for this firmware (4.6 and newer, up to 6.x)
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
            let firmware = fields.get(2).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            let package = nickelmenu_for(firmware.as_deref());
            KoboDevice {
                nickelmenu: nickelmenu_status(&root, firmware.as_deref()),
                nickelmenu_version: package.map(|p| p.version),
                nickelmenu_fork: package.is_some_and(|p| p.fork),
                firmware_supported: package.is_some(),
                firmware,
                model: model.map(|m| m.key.to_string()),
                model_name: model.map(|m| m.name.to_string()),
                model_id,
                installed: root.join(".kobo-ha/kobo-ha-dashboard").is_file(),
                has_config: root.join(".kobo-ha/config.yaml").is_file(),
                path: root.to_string_lossy().into_owned(),
            }
        })
        .collect()
}

/// Written by launch.sh: the Kobo software the dashboard last started on
/// from the menu, proof that NickelMenu worked there.
pub const LAUNCHED_ON: &str = ".kobo-ha/launched-on-firmware";

/// Update packages the Kobo installs when it restarts.
const PACKAGE_NAMES: [&str; 2] = ["KoboRoot.tgz", "Kobo.tgz"];

/// The NickelMenu release for a Kobo software version: the official one up to
/// 4.x, the Qt6 fork for 5.x and 6.x, none before 4.6 or after 6.x (not
/// known to work). An unreadable version gets the official release, as before.
pub fn nickelmenu_for(firmware: Option<&str>) -> Option<&'static paths::NickelMenuPackage> {
    let Some(version) = firmware else { return Some(&paths::NICKELMENU_QT5) };
    let mut parts = version.split('.').map(|p| p.trim().parse::<u32>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(4), Some(minor)) if minor >= 6 => Some(&paths::NICKELMENU_QT5),
        (Some(5 | 6), _) => Some(&paths::NICKELMENU_QT6),
        (Some(_), _) => None,
        (None, _) => Some(&paths::NICKELMENU_QT5),
    }
}

fn major(firmware: Option<&str>) -> Option<u32> {
    firmware?.split('.').next()?.trim().parse().ok()
}

/// `.adds/nm/doc` is written when NickelMenu is first installed (our own menu
/// entry also lives in `.adds/nm`, so the folder alone proves nothing). But a
/// Kobo software update removes NickelMenu and leaves that file behind, so
/// it is only trusted on the software the dashboard last started on. Without
/// that record, on 5.x and newer (only reached by updating, after which the
/// official release no longer works) it needs checking too.
fn nickelmenu_status(root: &Path, firmware: Option<&str>) -> &'static str {
    if PACKAGE_NAMES.iter().any(|n| root.join(".kobo").join(n).is_file()) {
        return "pending";
    }
    if !root.join(".adds/nm/doc").is_file() || root.join(".adds/nm/uninstall").exists() {
        return "missing";
    }
    let launched_on = std::fs::read_to_string(root.join(LAUNCHED_ON)).ok().map(|s| s.trim().to_string());
    match (launched_on, firmware) {
        (Some(last), Some(now)) if last != now => "check",
        (None, now) if major(now).is_some_and(|m| m >= 5) => "check",
        _ => "installed",
    }
}

/// Put a NickelMenu package where the Kobo picks up updates. It is
/// installed by the Kobo itself on the next restart.
pub fn install_nickelmenu(root: &Path, package: &[u8], file_name: &str) -> Result<(), String> {
    if !root.join(".kobo").is_dir() {
        return Err(format!("{} does not look like a Kobo (no .kobo folder)", root.display()));
    }
    let target = root.join(".kobo").join(file_name);
    // Don't overwrite some other pending update (e.g. another add-on).
    for name in PACKAGE_NAMES {
        if let Ok(existing) = std::fs::read(root.join(".kobo").join(name)) {
            if name != file_name || existing != package {
                return Err("other_package_pending".to_string());
            }
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
    std::fs::write(app.join("wifi.sh"), WIFI_SH).map_err(|e| err("Writing wifi.sh", e))?;
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
    fn kobo_scripts_have_unix_line_endings() {
        // CRLF is a syntax error for the Kobo's shell, before anything is
        // logged (issue #6). This test runs in CI on Windows too.
        assert!(!LAUNCH_SH.contains('\r'), "launch.sh has CRLF line endings");
        assert!(!WIFI_SH.contains('\r'), "wifi.sh has CRLF line endings");
    }

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
        assert!(root.join(".kobo-ha/wifi.sh").is_file());
        assert!(!root.join(".adds/nm/._kobo-ha").exists());
        assert_eq!(read_menu_name(&root).as_deref(), Some("My  Home"));

        // Our menu entry created .adds/nm, but NickelMenu itself is not installed
        assert_eq!(nickelmenu_status(&root, Some("4.38.21908")), "missing");
        install_nickelmenu(&root, b"pkg", "KoboRoot.tgz").unwrap();
        assert_eq!(nickelmenu_status(&root, Some("4.38.21908")), "pending");
        assert_eq!(install_nickelmenu(&root, b"other", "KoboRoot.tgz").unwrap_err(), "other_package_pending");
        assert_eq!(install_nickelmenu(&root, b"pkg", "Kobo.tgz").unwrap_err(), "other_package_pending");
        install_nickelmenu(&root, b"pkg", "KoboRoot.tgz").unwrap();
        // The Kobo installed it on restart
        std::fs::remove_file(root.join(".kobo/KoboRoot.tgz")).unwrap();
        std::fs::write(root.join(".adds/nm/doc"), "doc").unwrap();
        assert_eq!(nickelmenu_status(&root, Some("4.38.21908")), "installed");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_kobo_update_since_the_last_start_needs_nickelmenu_checked() {
        let root = std::env::temp_dir().join(format!("kobo-ha-test-nm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".kobo")).unwrap();
        std::fs::create_dir_all(root.join(".adds/nm")).unwrap();
        std::fs::create_dir_all(root.join(".kobo-ha")).unwrap();
        std::fs::write(root.join(".adds/nm/doc"), "doc").unwrap();
        // No record yet: 4.x trusts doc, 5.x and newer were reached by an update
        assert_eq!(nickelmenu_status(&root, Some("4.38.21908")), "installed");
        assert_eq!(nickelmenu_status(&root, Some("6.0.279225")), "check");
        // Started from the menu on this very software: fine
        std::fs::write(root.join(LAUNCHED_ON), "6.0.279225\n").unwrap();
        assert_eq!(nickelmenu_status(&root, Some("6.0.279225")), "installed");
        // Updated since (the case on the Clara Colour: 4.x to 6.0, menu gone)
        std::fs::write(root.join(LAUNCHED_ON), "4.43.23418\n").unwrap();
        assert_eq!(nickelmenu_status(&root, Some("6.0.279225")), "check");
        // Reinstalling: pending until the Kobo restarts
        install_nickelmenu(&root, b"qt6", "Kobo.tgz").unwrap();
        assert_eq!(nickelmenu_status(&root, Some("6.0.279225")), "pending");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn launch_sh_records_the_firmware_where_the_setup_tool_looks() {
        let name = LAUNCHED_ON.trim_start_matches(".kobo-ha/");
        assert!(LAUNCH_SH.contains(&format!(">\"$APP_DIR/{}\"", name)), "launch.sh must write {}", LAUNCHED_ON);
    }

    #[test]
    fn nickelmenu_release_follows_the_firmware() {
        let v = |f: Option<&str>| nickelmenu_for(f).map(|p| (p.file_name, p.fork));
        assert_eq!(v(Some("4.46.23836")), Some(("KoboRoot.tgz", false)));
        assert_eq!(v(Some("4.6.9960")), Some(("KoboRoot.tgz", false)));
        assert_eq!(v(Some("4.5.9587")), None);
        assert_eq!(v(Some("5.2.1")), Some(("Kobo.tgz", true)));
        assert_eq!(v(Some("6.0.279225")), Some(("Kobo.tgz", true)));
        assert_eq!(v(Some("7.0.1")), None, "not known to work yet");
        assert_eq!(v(None), Some(("KoboRoot.tgz", false)));
        assert_eq!(v(Some("garbled")), Some(("KoboRoot.tgz", false)));
    }
}
