//! "Is there a newer version?", answered with a single anonymous request.
//!
//! What is sent: an HTTPS GET for the public GitHub release list of this project, with a fixed
//! User-Agent (GitHub requires one) that carries no version, OS or identifier of any kind. No
//! cookies, no account, no token, and curl's config files are ignored. GitHub necessarily sees the
//! IP address of any web request; this app adds nothing to it and keeps no record of the answer
//! beyond the time of the last check, stored on this computer. See PRIVACY.md.
//!
//! curl (included in Windows 10+ and macOS) does the transfer, so no HTTP/TLS stack is linked in
//! and the system's certificate store and proxy settings apply, the same way ffmpeg is a subprocess.

use serde::{Deserialize, Serialize};
use std::process::Command;

const REPO: &str = env!("CARGO_PKG_REPOSITORY");
const USER_AGENT: &str = "recording-fixer-update-check";

#[derive(Serialize, Debug, PartialEq)]
pub struct UpdateInfo {
    pub current: String,
    /// The newest published release, or None when the project has not published one yet.
    pub latest: Option<String>,
    pub url: Option<String>,
    pub newer: bool,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// `(major, minor, patch)` and whether it carries a pre-release suffix ("1.2.3-beta.1").
fn parse_version(v: &str) -> Option<([u64; 3], bool)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let pre = v.contains('-');
    let core = v.split(['-', '+']).next().unwrap_or("");
    let mut parts = core.split('.');
    let mut out = [0u64; 3];
    for (i, slot) in out.iter_mut().enumerate() {
        match parts.next() {
            Some(p) => *slot = p.parse().ok()?,
            None if i == 0 => return None,
            None => break,
        }
    }
    if parts.next().is_some() {
        return None;
    }
    Some((out, pre))
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some((l, lpre)), Some((c, cpre))) => l > c || (l == c && cpre && !lpre),
        _ => false,
    }
}

fn releases_url() -> String {
    format!(
        "{}/releases/latest",
        REPO.replacen("https://github.com/", "https://api.github.com/repos/", 1)
    )
}

fn interpret(json: &str, current: &str) -> Result<UpdateInfo, String> {
    let release: Release =
        serde_json::from_str(json).map_err(|e| format!("unexpected reply: {e}"))?;
    // Only ever hand the UI a link that stays within this project.
    if !release.html_url.starts_with(&format!("{REPO}/")) {
        return Err("the release link points outside the project".into());
    }
    let newer = !release.draft && !release.prerelease && is_newer(&release.tag_name, current);
    Ok(UpdateInfo {
        current: current.into(),
        latest: Some(release.tag_name.trim_start_matches(['v', 'V']).into()),
        url: Some(release.html_url),
        newer,
    })
}

fn fetch() -> Result<Option<String>, String> {
    let mut cmd = Command::new("curl");
    cmd.args([
        "--disable", // ignore ~/.curlrc
        "--silent",
        "--show-error",
        "--fail",
        "--max-time",
        "15",
        "--max-filesize",
        "1000000",
        "--proto",
        "=https",
        "--location",
        "--proto-redir",
        "=https",
        "--user-agent",
        USER_AGENT,
        "--header",
        "Accept: application/vnd.github+json",
        "--header",
        "X-GitHub-Api-Version: 2022-11-28",
    ])
    .arg(releases_url());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "curl was not found on this computer".to_string()
        } else {
            e.to_string()
        }
    })?;
    if out.status.success() {
        return Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()));
    }
    let err = String::from_utf8_lossy(&out.stderr);
    // GitHub answers 404 until the first release exists. That is not a failure.
    if err.contains("404") {
        return Ok(None);
    }
    Err(err.lines().next().unwrap_or("request failed").trim().into())
}

pub fn check(current: &str) -> Result<UpdateInfo, String> {
    if std::env::var_os("RECORDING_FIXER_NO_UPDATE_CHECK").is_some() {
        return Err("update checks are switched off".into());
    }
    match fetch()? {
        Some(json) => interpret(&json, current),
        None => Ok(UpdateInfo {
            current: current.into(),
            latest: None,
            url: None,
            newer: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.5"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("v1", "1.0.0"));
    }

    #[test]
    fn a_release_beats_its_own_pre_release_but_not_the_reverse() {
        assert!(is_newer("1.0.0", "1.0.0-beta.1"));
        assert!(!is_newer("1.0.0-beta.1", "1.0.0"));
    }

    #[test]
    fn unreadable_versions_never_claim_an_update() {
        assert!(!is_newer("latest", "0.1.0"));
        assert!(!is_newer("1.2.3.4", "0.1.0"));
        assert!(!is_newer("", "0.1.0"));
    }

    fn reply(tag: &str, url: &str, extra: &str) -> String {
        format!(r#"{{"tag_name":"{tag}","html_url":"{url}"{extra},"body":"ignored"}}"#)
    }

    #[test]
    fn a_newer_release_is_reported_with_its_link() {
        let url = format!("{REPO}/releases/tag/v0.2.0");
        let info = interpret(&reply("v0.2.0", &url, ""), "0.1.0").unwrap();
        assert_eq!(
            info,
            UpdateInfo {
                current: "0.1.0".into(),
                latest: Some("0.2.0".into()),
                url: Some(url),
                newer: true
            }
        );
    }

    #[test]
    fn drafts_and_pre_releases_are_not_offered() {
        let url = format!("{REPO}/releases/tag/v0.2.0");
        for extra in [r#","draft":true"#, r#","prerelease":true"#] {
            assert!(
                !interpret(&reply("v0.2.0", &url, extra), "0.1.0")
                    .unwrap()
                    .newer
            );
        }
    }

    #[test]
    fn a_link_outside_the_project_is_refused() {
        let bad = reply("v9.0.0", "https://evil.example/download", "");
        assert!(interpret(&bad, "0.1.0").is_err());
        let lookalike = reply("v9.0.0", &format!("{REPO}-evil/releases"), "");
        assert!(interpret(&lookalike, "0.1.0").is_err());
    }

    #[test]
    fn junk_replies_are_errors_not_updates() {
        assert!(interpret("not json", "0.1.0").is_err());
        assert!(interpret("{}", "0.1.0").is_err());
    }

    #[test]
    fn the_request_goes_to_this_projects_public_release_list() {
        assert_eq!(
            releases_url(),
            "https://api.github.com/repos/djryanj/recording-fixer/releases/latest"
        );
    }
}
