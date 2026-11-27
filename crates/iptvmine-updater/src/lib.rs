use reqwest::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UpdaterError {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("GitHub API returned status {0}")]
    ApiStatus(u16),
    #[error("invalid semver tag: {0}")]
    SemVer(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
    #[serde(default)]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubRelease {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_name: String,
    pub release_notes: String,
    pub release_url: String,
    pub download_url: Option<String>,
    pub asset_name: Option<String>,
}

pub async fn check_latest_release(
    client: &Client,
    owner: &str,
    repo: &str,
    current_version: &str,
) -> Result<UpdateCheckResult, UpdaterError> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(UpdaterError::ApiStatus(resp.status().as_u16()));
    }

    let release: GithubRelease = resp.json().await?;
    evaluate_release(&release, current_version)
}

pub fn evaluate_release(
    release: &GithubRelease,
    current_version: &str,
) -> Result<UpdateCheckResult, UpdaterError> {
    let latest_clean = release
        .tag_name
        .trim()
        .trim_start_matches(['v', 'V'])
        .to_string();
    let current_clean = current_version.trim().trim_start_matches(['v', 'V']);

    let latest_ver =
        Version::parse(&latest_clean).map_err(|e| UpdaterError::SemVer(e.to_string()))?;
    let current_ver =
        Version::parse(current_clean).map_err(|e| UpdaterError::SemVer(e.to_string()))?;

    let asset = select_platform_asset(&release.assets);

    Ok(UpdateCheckResult {
        current_version: current_ver.to_string(),
        latest_version: latest_ver.to_string(),
        update_available: latest_ver > current_ver,
        release_name: release
            .name
            .clone()
            .unwrap_or_else(|| release.tag_name.clone()),
        release_notes: release.body.clone().unwrap_or_default(),
        release_url: release.html_url.clone().unwrap_or_default(),
        download_url: asset.map(|a| a.browser_download_url.clone()),
        asset_name: asset.map(|a| a.name.clone()),
    })
}

pub fn select_platform_asset(assets: &[GithubAsset]) -> Option<&GithubAsset> {
    select_platform_asset_for(assets, std::env::consts::OS, std::env::consts::ARCH)
}

pub fn select_platform_asset_for<'a>(
    assets: &'a [GithubAsset],
    os: &str,
    arch: &str,
) -> Option<&'a GithubAsset> {
    if os == "windows" {
        let is_arm64 = arch == "aarch64";
        let preferred_exts: &[&str] = if is_arm64 {
            &[".exe", ".msi"]
        } else {
            &[".msi", ".exe"]
        };

        // 1. Prefer exact architecture match first (e.g. *_arm64-setup.exe on ARM64, *_x64* on x86_64)
        for ext in preferred_exts {
            if let Some(found) = assets.iter().find(|a| {
                let lower = a.name.to_ascii_lowercase();
                if !lower.ends_with(ext) {
                    return false;
                }
                let has_arm = lower.contains("arm64") || lower.contains("aarch64");
                if is_arm64 {
                    has_arm
                } else {
                    !has_arm
                }
            }) {
                return Some(found);
            }
        }
    }

    let preferred_exts: &[&str] = match os {
        "windows" => &[".msi", ".exe"],
        "macos" => &[".dmg", ".app.tar.gz"],
        _ => &[".appimage", ".deb"],
    };

    for ext in preferred_exts {
        if let Some(found) = assets
            .iter()
            .find(|a| a.name.to_ascii_lowercase().ends_with(ext))
        {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_semver_and_selects_platform_asset() {
        let release = GithubRelease {
            tag_name: "v1.2.0".into(),
            name: Some("Desktop v1.2.0".into()),
            body: Some("Release notes".into()),
            prerelease: false,
            html_url: Some("https://github.com/samyak2403/IPTVMine-Pro/releases/tag/v1.2.0".into()),
            assets: vec![
                GithubAsset {
                    name: "IPTVMine-Desktop.msi".into(),
                    browser_download_url: "https://example.com/app.msi".into(),
                    size: 100,
                    content_type: None,
                },
                GithubAsset {
                    name: "IPTVMine-Desktop.dmg".into(),
                    browser_download_url: "https://example.com/app.dmg".into(),
                    size: 100,
                    content_type: None,
                },
                GithubAsset {
                    name: "IPTVMine-Desktop.AppImage".into(),
                    browser_download_url: "https://example.com/app.AppImage".into(),
                    size: 100,
                    content_type: None,
                },
            ],
        };

        let res = evaluate_release(&release, "0.1.0").unwrap();
        assert!(res.update_available);
        assert!(res.download_url.is_some());
    }

    #[test]
    fn selects_arm64_and_x64_windows_assets_accurately() {
        let assets = vec![
            GithubAsset {
                name: "IPTVMine Desktop_0.2.0_x64_en-US.msi".into(),
                browser_download_url: "https://example.com/x64.msi".into(),
                size: 100,
                content_type: None,
            },
            GithubAsset {
                name: "IPTVMine Desktop_0.2.0_x64-setup.exe".into(),
                browser_download_url: "https://example.com/x64-setup.exe".into(),
                size: 100,
                content_type: None,
            },
            GithubAsset {
                name: "IPTVMine Desktop_0.2.0_arm64-setup.exe".into(),
                browser_download_url: "https://example.com/arm64-setup.exe".into(),
                size: 100,
                content_type: None,
            },
        ];

        assert_eq!(
            select_platform_asset_for(&assets, "windows", "aarch64")
                .map(|a| a.name.as_str()),
            Some("IPTVMine Desktop_0.2.0_arm64-setup.exe")
        );
        assert_eq!(
            select_platform_asset_for(&assets, "windows", "x86_64")
                .map(|a| a.name.as_str()),
            Some("IPTVMine Desktop_0.2.0_x64_en-US.msi")
        );
    }
}
