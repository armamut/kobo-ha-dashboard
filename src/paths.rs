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

/// NickelMenu release offered by the setup tool, pinned and checksummed.
pub const NICKELMENU_VERSION: &str = "v0.6.0";
const NICKELMENU_URL: &str = "https://github.com/pgaskin/NickelMenu/releases/download/v0.6.0/KoboRoot.tgz";
const NICKELMENU_SHA256: &str = "322ff9aa863860e8f5f7e0b55cae561c54bf95983b9bce1d19819d1225d064af";

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{:02x}", b)).collect()
}

/// NickelMenu's KoboRoot.tgz, from the cache or downloaded and verified.
pub async fn nickelmenu_package() -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("NickelMenu-{}-KoboRoot.tgz", NICKELMENU_VERSION));
    if let Ok(bytes) = std::fs::read(&path) {
        if sha256_hex(&bytes) == NICKELMENU_SHA256 {
            return Ok(bytes);
        }
    }
    tracing::info!("Downloading {}", NICKELMENU_URL);
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)).build()?;
    let resp = client.get(NICKELMENU_URL).send().await?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()).into());
    }
    let bytes = resp.bytes().await?.to_vec();
    if sha256_hex(&bytes) != NICKELMENU_SHA256 {
        return Err("the downloaded file does not match the expected checksum".into());
    }
    std::fs::write(&path, &bytes)?;
    Ok(bytes)
}
