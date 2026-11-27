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
    let preferred_exts: &[&str] = if cfg!(target_os = "windows") {
        &[".msi", ".exe"]
    } else if cfg!(target_os = "macos") {
        &[".dmg", ".app.tar.gz"]
    } else {
        &[".appimage", ".deb"]
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
}
