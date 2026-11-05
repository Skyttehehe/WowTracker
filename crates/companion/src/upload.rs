//! Sends payloads to the backend's `POST /ingest`.

use std::time::Duration;
use wowtracker_protocol::IngestPayload;

/// Turns `192.168.0.117:1337`, `http://host:1337` or a full URL into the ingest URL.
pub fn ingest_url(endpoint: &str) -> Result<String, String> {
    let e = endpoint.trim();
    if e.is_empty() {
        return Err("no backend address set".into());
    }
    let with_scheme = if e.contains("://") { e.to_string() } else { format!("http://{e}") };
    let (scheme, rest) = with_scheme.split_once("://").unwrap();
    if !matches!(scheme, "http" | "https") {
        return Err(format!("unsupported scheme '{scheme}' in '{e}'"));
    }
    if rest.is_empty() || rest.starts_with('/') {
        return Err(format!("no host in '{e}'"));
    }
    match rest.find('/') {
        None => Ok(format!("{scheme}://{rest}/ingest")),
        Some(i) if &rest[i..] == "/" => Ok(format!("{scheme}://{}/ingest", &rest[..i])),
        Some(_) => Ok(with_scheme),
    }
}

pub fn send(url: &str, token: Option<&str>, payload: &IngestPayload) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
    let mut req = agent.post(url);
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    match req.send_json(payload) {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(401, _)) => Err("backend rejected the token (401). Check /wt token or token in the config".into()),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("backend answered {code}: {}", body.trim()))
        }
        Err(e) => Err(format!("cannot reach {url}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_ingest_urls() {
        assert_eq!(ingest_url("192.168.0.117:1337").unwrap(), "http://192.168.0.117:1337/ingest");
        assert_eq!(ingest_url("http://192.168.0.117:1337/").unwrap(), "http://192.168.0.117:1337/ingest");
        assert_eq!(ingest_url("https://ingest.example.com").unwrap(), "https://ingest.example.com/ingest");
        assert_eq!(ingest_url("https://example.com/wt/ingest").unwrap(), "https://example.com/wt/ingest");
        assert!(ingest_url("").is_err());
        assert!(ingest_url("ftp://x").is_err());
    }
}
