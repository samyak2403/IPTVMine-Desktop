use std::net::IpAddr;
use url::Url;

/// Validates outbound scraper HTTP URLs against SSRF rules from SECURITY_AND_ACCESS.md.
/// Blocks non-HTTP(S) schemes, loopback, link-local, and RFC1918 private IPs unless
/// `developer_test_mode` is explicitly enabled.
pub fn validate_scraper_url(raw: &str, developer_test_mode: bool) -> Result<Url, String> {
    let parsed = Url::parse(raw.trim()).map_err(|e| format!("invalid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("blocked URL scheme: {other}")),
    }

    let Some(host) = parsed.host_str() else {
        return Err("URL missing host".into());
    };

    if developer_test_mode {
        return Ok(parsed);
    }

    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") || lower.ends_with(".local") {
        return Err(format!("blocked local host: {host}"));
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_restricted_ip(ip) {
            return Err(format!("blocked private/loopback IP: {ip}"));
        }
    }

    Ok(parsed)
}

fn is_restricted_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified() || v6.is_unique_local(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_ssrf_unless_developer_test_mode() {
        assert!(validate_scraper_url("file:///etc/passwd", false).is_err());
        assert!(validate_scraper_url("http://127.0.0.1:3001/dist", false).is_err());
        assert!(validate_scraper_url("http://169.254.169.254/latest", false).is_err());
        assert!(validate_scraper_url("http://192.168.1.10:3001/dist", false).is_err());
        assert!(validate_scraper_url("http://192.168.1.10:3001/dist", true).is_ok());
        assert!(validate_scraper_url("https://example.com/api", false).is_ok());
    }
}
