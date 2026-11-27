use crate::ProviderType;
use url::Url;

const VIDEO_EXTENSIONS: &[&str] = &[
    ".mp4", ".mkv", ".avi", ".mov", ".wmv", ".flv", ".webm", ".mpg", ".mpeg", ".3gp", ".ogv",
    ".m4v", ".rm", ".rmvb",
];

pub fn is_direct_video_url(raw_url: &str) -> bool {
    let lower = raw_url.trim().to_ascii_lowercase();
    let path = lower
        .split('?')
        .next()
        .unwrap_or(&lower)
        .split('#')
        .next()
        .unwrap_or(&lower);
    VIDEO_EXTENSIONS.iter().any(|ext| path.ends_with(ext))
}

pub fn is_valid_stream_scheme(raw_url: &str) -> bool {
    let lower = raw_url.trim().to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("rtmp://")
        || lower.starts_with("rtmps://")
        || lower.starts_with("rtsp://")
        || lower.starts_with("udp://")
        || lower.starts_with("rtp://")
}

/// Fast URL/title heuristic mirroring Android `ProviderDetectionEngine.detectQuick`.
pub fn detect_provider_quick(input_url: &str, title: &str) -> ProviderType {
    let trimmed = input_url.trim();
    let lower = trimmed.to_ascii_lowercase();
    let path = lower
        .split('?')
        .next()
        .unwrap_or(&lower)
        .split('#')
        .next()
        .unwrap_or(&lower);

    if is_direct_video_url(trimmed) {
        ProviderType::Video
    } else if path.ends_with(".m3u") || path.ends_with(".m3u8") {
        ProviderType::Iptv
    } else if trimmed.starts_with('@')
        || lower.contains("vega")
        || title.to_ascii_lowercase().contains("vega")
        || looks_like_vega_repo_or_json(path)
    {
        ProviderType::Vega
    } else {
        ProviderType::Iptv
    }
}

fn looks_like_vega_repo_or_json(lower_path: &str) -> bool {
    if lower_path.ends_with("/manifest.json")
        || lower_path.ends_with("/urls.json")
        || lower_path.ends_with("/modflix.json")
    {
        return true;
    }
    (lower_path.starts_with("https://github.com/") || lower_path.starts_with("http://github.com/"))
        && !lower_path.ends_with(".m3u")
}

/// Derives a readable provider title from a URL or `@handle`.
pub fn derive_provider_title(raw_url: &str) -> String {
    let trimmed = raw_url.trim().trim_end_matches('/');
    if let Some(handle) = trimmed.strip_prefix('@') {
        return if handle.is_empty() {
            "Provider".to_string()
        } else {
            handle.to_string()
        };
    }

    if let Ok(parsed) = Url::parse(trimmed) {
        let segments: Vec<&str> = parsed
            .path_segments()
            .map(|s| s.filter(|p| !p.is_empty()).collect())
            .unwrap_or_default();
        let generic = [
            "index",
            "playlist",
            "manifest",
            "urls",
            "modflix",
            "channels",
        ];
        if segments.len() > 1 {
            if let Some(last) = segments.last() {
                let base = last.split('.').next().unwrap_or(last);
                if !base.is_empty() && !generic.contains(&base.to_ascii_lowercase().as_str()) {
                    return base.to_string();
                }
            }
            if let Some(parent) = segments.get(segments.len() - 2) {
                if !parent.is_empty() {
                    return (*parent).to_string();
                }
            }
        }
        if let Some(host) = parsed.host_str() {
            let clean_host = host.trim_start_matches("www.");
            if !clean_host.is_empty() {
                return clean_host.to_string();
            }
        }
    }

    "Provider".to_string()
}

/// Sanitizes a download title into a safe filename stem (no traversal or OS-forbidden chars).
pub fn sanitize_filename(title: &str) -> String {
    let cleaned = crate::clean_title(title);
    let sanitized: String = cleaned
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect();
    let trimmed = sanitized.trim_matches('.').trim();
    if trimmed.is_empty() {
        "video".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_detection_matches_android_heuristics() {
        assert_eq!(
            detect_provider_quick("https://example.com/live.m3u8", ""),
            ProviderType::Iptv
        );
        assert_eq!(
            detect_provider_quick("https://example.com/movie.mkv", ""),
            ProviderType::Video
        );
        assert_eq!(
            detect_provider_quick("https://raw.githubusercontent.com/user/repo/main/manifest.json", ""),
            ProviderType::Vega
        );
        assert_eq!(
            detect_provider_quick("@vega-org", ""),
            ProviderType::Vega
        );
    }

    #[test]
    fn sanitize_filename_removes_traversal_and_illegal_chars() {
        assert_eq!(sanitize_filename("../evil:name*?.mp4"), "_evil_name__.mp4");
        assert_eq!(sanitize_filename("..."), "video");
    }
}
