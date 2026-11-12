//! Per-user directories for the desktop setup tool, and font locations.

use std::path::{Path, PathBuf};

const APP: &str = "kobo-ha-dashboard";

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Where the setup tool keeps its working `config.yaml`.
pub fn data_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support").join(APP)
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(home).join(APP)
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config")).join(APP)
    }
}

/// Where downloaded fonts are cached.
pub fn cache_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        home().join("Library/Caches").join(APP)
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(home).join(APP)
    } else {
        std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".cache")).join(APP)
    }
}

/// Directories searched for fallback (CJK) fonts.
pub fn font_dirs(config_dir: &Path) -> Vec<PathBuf> {
    vec![config_dir.join("fonts"), cache_dir().join("fonts")]
}

/// Download URL of a font file such as "NotoSansSC-Regular".
pub fn font_url(file_stem: &str) -> Option<String> {
    let script = match file_stem.split('-').next()? {
        "NotoSansSC" => "SC",
        "NotoSansJP" => "JP",
        "NotoSansTC" => "TC",
        "NotoSansKR" => "KR",
        _ => return None,
    };
    Some(format!("https://github.com/notofonts/noto-cjk/raw/main/Sans/SubsetOTF/{}/{}.otf", script, file_stem))
}

/// Make sure `<family>-Regular.otf` and `-Bold.otf` are in the font cache,
/// downloading them if needed. Returns the cached file paths.
pub async fn ensure_font(family: &str) -> Result<Vec<PathBuf>, Box<dyn std::error::Error + Send + Sync>> {
    let dir = cache_dir().join("fonts");
    std::fs::create_dir_all(&dir)?;
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(300)).build()?;
    let mut out = Vec::new();
    for weight in ["Regular", "Bold"] {
        let stem = format!("{}-{}", family, weight);
        let path = dir.join(format!("{}.otf", stem));
        if !path.exists() {
            let url = font_url(&stem).ok_or_else(|| format!("No download source for font {}", stem))?;
            tracing::info!("Downloading {}", url);
            let resp = client.get(&url).send().await?;
            if !resp.status().is_success() {
                return Err(format!("Font download failed: HTTP {}", resp.status()).into());
            }
            let bytes = resp.bytes().await?;
            let tmp = path.with_extension("part");
            std::fs::write(&tmp, &bytes)?;
            std::fs::rename(&tmp, &path)?;
        }
        out.push(path);
    }
    Ok(out)
}

/// A NickelMenu release offered by the setup tool, pinned and checksummed.
pub struct NickelMenuPackage {
    /// Shown in the setup tool and its log
    pub version: &'static str,
    url: &'static str,
    sha256: &'static str,
    /// Name in `.kobo/`, where the Kobo installs it like an update
    pub file_name: &'static str,
    /// Not the official release (Kobo software 5.x/6.x)
    pub fork: bool,
}

/// The official release, for Kobo software 4.6 to 4.x (Qt5).
pub const NICKELMENU_QT5: NickelMenuPackage = NickelMenuPackage {
    version: "v0.6.0",
    url: "https://github.com/pgaskin/NickelMenu/releases/download/v0.6.0/KoboRoot.tgz",
    sha256: "322ff9aa863860e8f5f7e0b55cae561c54bf95983b9bce1d19819d1225d064af",
    file_name: "KoboRoot.tgz",
    fork: false,
};

/// Kobo software 5.x and 6.x (Qt6) isn't supported by the official release
/// yet. Nico Verbruggen's fork builds it for Qt6 from the same source, checked
/// against 6.0.279225; its package installs to a different path, hence the
/// other file name.
pub const NICKELMENU_QT6: NickelMenuPackage = NickelMenuPackage {
    version: "fork v1.2 (Qt6)",
    url: "https://github.com/nicoverbruggen/NickelMenu/releases/download/fork-v1.2/Kobo.tgz",
    sha256: "2ec56131888d39fdb3842b7b086da839f83da6f17ffa3b69e758dc6bcbdaa3ce",
    file_name: "Kobo.tgz",
    fork: true,
};

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{:02x}", b)).collect()
}

/// A NickelMenu package, from the cache or downloaded and verified.
pub async fn nickelmenu_package(package: &NickelMenuPackage) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("NickelMenu-{}", package.sha256));
    if let Ok(bytes) = std::fs::read(&path) {
        if sha256_hex(&bytes) == package.sha256 {
            return Ok(bytes);
        }
    }
    tracing::info!("Downloading {}", package.url);
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)).build()?;
    let resp = client.get(package.url).send().await?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()).into());
    }
    let bytes = resp.bytes().await?.to_vec();
    if sha256_hex(&bytes) != package.sha256 {
        return Err("the downloaded file does not match the expected checksum".into());
    }
    std::fs::write(&path, &bytes)?;
    Ok(bytes)
}
