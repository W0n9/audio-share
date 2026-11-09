//! GitHub release update check.
//!
//! The comparison algorithm is frozen to v0.3.4's `util::is_newer_version`:
//! both versions must start with `v`, split into exactly three numeric
//! components, and compare component-wise. JSON parsing is separated from the
//! network so it can be tested with injected GitHub responses.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Latest-release endpoint used by v0.3.4 and this rewrite.
pub const API_URL: &str = "https://api.github.com/repos/mkckr0/audio-share/releases/latest";

/// Network timeout for the release request.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The version compiled into this build (`0.4.0`).
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The user agent GitHub requires for unauthenticated requests.
pub const USER_AGENT: &str = concat!("AudioShareServer/", env!("CARGO_PKG_VERSION"));

/// How often an automatic update check runs (v0.3.4 parity).
///
/// The first automatic check happens one full interval after launch; a startup
/// check is intentionally not performed. The manual "Check Now" path is
/// independent of this timer.
pub const AUTOMATIC_CHECK_INTERVAL: Duration = Duration::from_secs(3 * 60 * 60);

/// Whether an automatic timer tick should perform a check.
///
/// The `AutoUpdate` setting is re-read on every tick, so toggling it takes
/// effect without restarting the GUI.
pub const fn should_run_automatic_check(auto_update: bool) -> bool {
    auto_update
}

/// The fields this app consumes from a GitHub release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseInfo {
    /// Raw `tag_name`, e.g. `v0.4.1`.
    pub tag_name: String,
    /// Release page URL opened when the user accepts the notification.
    pub html_url: String,
}

/// Outcome of one update check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum UpdateCheckResult {
    /// The running version is current.
    UpToDate,
    /// A release newer than [`CURRENT_VERSION`] exists.
    Newer {
        /// Raw remote tag, e.g. `v0.4.1`.
        version: String,
        /// Release page URL.
        url: String,
    },
}

/// Parse a GitHub `releases/latest` response body.
pub fn parse_release(body: &str) -> Result<ReleaseInfo, String> {
    #[derive(Deserialize)]
    struct RawRelease {
        tag_name: Option<String>,
        html_url: Option<String>,
    }

    let raw: RawRelease =
        serde_json::from_str(body).map_err(|error| format!("invalid release response: {error}"))?;
    let tag_name = raw
        .tag_name
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "invalid release response: missing tag_name".to_owned())?;
    let html_url = raw
        .html_url
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "invalid release response: missing html_url".to_owned())?;
    Ok(ReleaseInfo { tag_name, html_url })
}

/// Apply v0.3.4's `is_newer_version(remote, local)` algorithm.
///
/// Both strings must start with `v`. They are split on `.` with empty
/// components ignored, exactly three components are required, and each is
/// parsed like C++ `std::stoi`: leading whitespace and sign are accepted and
/// trailing non-numeric text is ignored. The result is `remote > local`.
pub fn is_newer_version(remote: &str, local: &str) -> Result<bool, String> {
    let remote_parts = parse_version(remote)?;
    let local_parts = parse_version(local)?;
    for (remote, local) in remote_parts.iter().zip(local_parts.iter()) {
        if remote != local {
            return Ok(remote > local);
        }
    }
    Ok(false)
}

/// Parse one `v`-prefixed version into exactly three integer components.
fn parse_version(version: &str) -> Result<[i32; 3], String> {
    let body = version
        .strip_prefix('v')
        .ok_or_else(|| format!("invalid version {version:?}: must start with 'v'"))?;
    let parts: Vec<&str> = body.split('.').filter(|part| !part.is_empty()).collect();
    if parts.len() != 3 {
        return Err(format!(
            "invalid version {version:?}: expected 3 numeric components, found {}",
            parts.len()
        ));
    }
    let mut parsed = [0i32; 3];
    for (index, part) in parts.iter().enumerate() {
        parsed[index] = parse_leading_int(part).ok_or_else(|| {
            format!("invalid version {version:?}: component {part:?} is not numeric")
        })?;
    }
    Ok(parsed)
}

/// C++ `std::stoi`-style leading integer parse, including overflow rejection.
fn parse_leading_int(value: &str) -> Option<i32> {
    let trimmed = value.trim_start_matches(|character: char| character.is_ascii_whitespace());
    let mut characters = trimmed.chars().peekable();
    let negative = match characters.peek() {
        Some('-') => {
            characters.next();
            true
        }
        Some('+') => {
            characters.next();
            false
        }
        _ => false,
    };
    let mut digits = 0usize;
    let mut magnitude: i64 = 0;
    while let Some(character) = characters.peek().copied() {
        if !character.is_ascii_digit() {
            break;
        }
        characters.next();
        digits += 1;
        magnitude = magnitude
            .checked_mul(10)?
            .checked_add((character as u8 - b'0') as i64)?;
        if magnitude > i32::MAX as i64 + 1 {
            return None;
        }
    }
    if digits == 0 {
        return None;
    }
    let signed = if negative { -magnitude } else { magnitude };
    i32::try_from(signed).ok()
}

/// Parse a release body and compare its tag against `local`.
pub fn evaluate_release(body: &str, local: &str) -> Result<UpdateCheckResult, String> {
    let release = parse_release(body)?;
    if is_newer_version(&release.tag_name, local)? {
        Ok(UpdateCheckResult::Newer {
            version: release.tag_name,
            url: release.html_url,
        })
    } else {
        Ok(UpdateCheckResult::UpToDate)
    }
}

/// Interpret an HTTP status and body from the release endpoint.
pub fn interpret_response(
    status: u16,
    body: &str,
    local: &str,
) -> Result<UpdateCheckResult, String> {
    if !(200..=299).contains(&status) {
        let message = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|value| value.get("message")?.as_str().map(str::to_owned))
            .filter(|message| !message.trim().is_empty());
        return Err(match message {
            Some(message) => format!("update check failed: HTTP {status} ({message})"),
            None => format!("update check failed: HTTP {status}"),
        });
    }
    evaluate_release(body, local)
}

/// Fetch and evaluate the latest release. Blocking; call off the UI thread.
pub fn fetch_latest(url: &str) -> Result<UpdateCheckResult, String> {
    let agent = ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build();
    let local = format!("v{CURRENT_VERSION}");
    match agent.get(url).set("User-Agent", USER_AGENT).call() {
        Ok(response) => {
            let status = response.status();
            let body = response
                .into_string()
                .map_err(|error| format!("update check failed: cannot read response: {error}"))?;
            interpret_response(status, &body, &local)
        }
        // ureq 2 reports non-2xx statuses as Err(Status), keeping the body so
        // rate-limit messages can be shown readably.
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            interpret_response(status, &body, &local)
        }
        Err(error) => Err(format!("update check failed: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEWER_BODY: &str = r#"{
        "url": "https://api.github.com/repos/mkckr0/audio-share/releases/1",
        "html_url": "https://github.com/mkckr0/audio-share/releases/tag/v0.4.1",
        "tag_name": "v0.4.1",
        "name": "Audio Share 0.4.1",
        "draft": false,
        "prerelease": false
    }"#;

    const SAME_BODY: &str = r#"{
        "html_url": "https://github.com/mkckr0/audio-share/releases/tag/v0.4.0",
        "tag_name": "v0.4.0"
    }"#;

    const OLDER_BODY: &str = r#"{
        "html_url": "https://github.com/mkckr0/audio-share/releases/tag/v0.3.4",
        "tag_name": "v0.3.4"
    }"#;

    #[test]
    fn automatic_checks_wait_one_full_interval_and_follow_the_setting() {
        assert_eq!(
            AUTOMATIC_CHECK_INTERVAL,
            Duration::from_secs(3 * 60 * 60),
            "v0.3.4 performs the first automatic check one interval after launch"
        );
        assert!(should_run_automatic_check(true));
        assert!(!should_run_automatic_check(false));
    }

    #[test]
    fn compares_only_remote_greater_than_local() {
        assert!(!is_newer_version("v0.4.0", "v0.4.0").unwrap());
        assert!(is_newer_version("v0.4.1", "v0.4.0").unwrap());
        assert!(is_newer_version("v0.5.0", "v0.4.9").unwrap());
        assert!(is_newer_version("v1.0.0", "v0.99.99").unwrap());
        assert!(!is_newer_version("v0.3.9", "v0.4.0").unwrap());
        assert!(!is_newer_version("v0.4.0", "v0.5.0").unwrap());
    }

    #[test]
    fn components_are_numeric_and_leading_zeroes_are_accepted() {
        assert!(is_newer_version("v0.4.01", "v0.4.0").unwrap());
        assert!(is_newer_version("v01.0.0", "v0.99.99").unwrap());
        assert!(!is_newer_version("v0.04.0", "v0.4.0").unwrap());
    }

    #[test]
    fn numeric_prefixes_are_accepted_like_std_stoi() {
        // The old C++ implementation used std::stoi, which parses a leading
        // integer and ignores the rest of the component.
        assert!(is_newer_version("v0.5.0-beta", "v0.4.0").unwrap());
        assert!(!is_newer_version("v0.4.0-beta", "v0.4.0").unwrap());
        assert!(is_newer_version("v+0.4.+1", "v0.4.0").unwrap());
        assert!(!is_newer_version("v 0.4.0", "v0.4.0").unwrap());
    }

    #[test]
    fn empty_components_are_ignored_like_the_old_splitter() {
        assert!(is_newer_version("v0..5.0", "v0.4.0").unwrap());
        assert!(is_newer_version("v.0.4.1.", "v0.4.0").unwrap());
    }

    #[test]
    fn malformed_versions_are_rejected_with_readable_errors() {
        for version in [
            "",
            "0.4.0",
            "v0.4",
            "v0.4.0.1",
            "vabc.2.3",
            "v1.x.3",
            "v1.2.3x.4",
        ] {
            let error = is_newer_version(version, "v0.4.0")
                .expect_err(&format!("{version:?} must be rejected"));
            assert!(
                error.to_lowercase().contains("version"),
                "error for {version:?} was {error:?}"
            );
        }
        assert!(is_newer_version("v0.5.0", "0.4.0").is_err());
    }

    #[test]
    fn parses_the_fields_the_app_needs() {
        let release = parse_release(NEWER_BODY).expect("valid release JSON");
        assert_eq!(release.tag_name, "v0.4.1");
        assert_eq!(
            release.html_url,
            "https://github.com/mkckr0/audio-share/releases/tag/v0.4.1"
        );
    }

    #[test]
    fn rejects_malformed_json_and_missing_fields() {
        for body in [
            "not json",
            "{}",
            r#"{"tag_name":"v0.5.0"}"#,
            r#"{"html_url":"x"}"#,
        ] {
            let error = parse_release(body).expect_err("invalid release JSON");
            assert!(
                error.contains("release"),
                "error for {body:?} was {error:?}"
            );
        }
    }

    #[test]
    fn evaluates_newer_same_and_older_releases() {
        assert_eq!(
            evaluate_release(NEWER_BODY, "v0.4.0").unwrap(),
            UpdateCheckResult::Newer {
                version: "v0.4.1".to_owned(),
                url: "https://github.com/mkckr0/audio-share/releases/tag/v0.4.1".to_owned(),
            }
        );
        assert_eq!(
            evaluate_release(SAME_BODY, "v0.4.0").unwrap(),
            UpdateCheckResult::UpToDate
        );
        assert_eq!(
            evaluate_release(OLDER_BODY, "v0.4.0").unwrap(),
            UpdateCheckResult::UpToDate
        );
    }

    #[test]
    fn successful_http_responses_are_evaluated() {
        assert_eq!(
            interpret_response(200, NEWER_BODY, "v0.4.0").unwrap(),
            UpdateCheckResult::Newer {
                version: "v0.4.1".to_owned(),
                url: "https://github.com/mkckr0/audio-share/releases/tag/v0.4.1".to_owned(),
            }
        );
        assert_eq!(
            interpret_response(200, SAME_BODY, "v0.4.0").unwrap(),
            UpdateCheckResult::UpToDate
        );
    }

    #[test]
    fn http_errors_include_status_and_github_message() {
        let error = interpret_response(
            403,
            r#"{"message":"API rate limit exceeded for 1.2.3.4."}"#,
            "v0.4.0",
        )
        .expect_err("403 is an error");
        assert!(error.contains("403"), "error was {error}");
        assert!(error.contains("rate limit"), "error was {error}");

        let error = interpret_response(500, "", "v0.4.0").expect_err("500 is an error");
        assert!(error.contains("500"), "error was {error}");
    }

    #[test]
    fn serializes_for_the_frontend_contract() {
        let json = serde_json::to_value(UpdateCheckResult::UpToDate).unwrap();
        assert_eq!(json["status"], "upToDate");

        let json = serde_json::to_value(UpdateCheckResult::Newer {
            version: "v0.4.1".to_owned(),
            url: "https://example.test/v0.4.1".to_owned(),
        })
        .unwrap();
        assert_eq!(json["status"], "newer");
        assert_eq!(json["version"], "v0.4.1");
        assert_eq!(json["url"], "https://example.test/v0.4.1");
    }
}
