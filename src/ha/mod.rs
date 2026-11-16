pub mod battery;
pub mod client;
pub mod mock;
pub mod models;
pub mod ws;

pub use client::HaClient;
pub use models::EntityState;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Home Assistant serves its API only at the root, so keep scheme, host and
/// port and drop page paths such as `/lovelace/0`. The flag tells whether
/// anything was dropped.
pub fn base_url(url: &str) -> (String, bool) {
    let trimmed = url.trim().trim_end_matches('/');
    match reqwest::Url::parse(trimmed) {
        Ok(u) if u.has_host() && matches!(u.scheme(), "http" | "https") => {
            let mut root = format!("{}://{}", u.scheme(), u.host_str().unwrap_or(""));
            if let Some(port) = u.port() {
                root.push_str(&format!(":{}", port));
            }
            let dropped = !u.path().trim_end_matches('/').is_empty() || u.query().is_some() || u.fragment().is_some();
            (root, dropped)
        }
        _ => (trimmed.to_string(), false),
    }
}

/// Decode a JSON response; when HA sends something else (its web app, a
/// proxy's error page) say what came back instead of reqwest's opaque
/// "error decoding response body".
pub async fn read_json<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> Result<T, BoxError> {
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("no content type")
        .to_string();
    let body = resp.text().await?;
    serde_json::from_str(&body).map_err(|e| {
        if content_type.contains("json") {
            format!("Home Assistant sent invalid JSON (HTTP {}): {}", status, e).into()
        } else {
            format!(
                "Home Assistant answered HTTP {} with {} instead of JSON. Use the root URL (like http://192.168.1.10:8123) without a page path.",
                status, content_type
            )
            .into()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::base_url;

    #[test]
    fn base_url_keeps_only_the_root() {
        assert_eq!(base_url("http://192.168.1.10:8123"), ("http://192.168.1.10:8123".into(), false));
        assert_eq!(base_url(" http://192.168.1.10:8123/ "), ("http://192.168.1.10:8123".into(), false));
        assert_eq!(base_url("http://192.168.1.10:8123/home/overview"), ("http://192.168.1.10:8123".into(), true));
        assert_eq!(base_url("http://192.168.1.10:8123/lovelace/0?edit=1"), ("http://192.168.1.10:8123".into(), true));
        assert_eq!(base_url("https://ha.example.com/"), ("https://ha.example.com".into(), false));
        assert_eq!(base_url("https://ha.example.com:443/x"), ("https://ha.example.com".into(), true));
        assert_eq!(base_url("http://[fd00::1]:8123/x"), ("http://[fd00::1]:8123".into(), true));
        assert_eq!(base_url("ha.local"), ("ha.local".into(), false));
    }
}
