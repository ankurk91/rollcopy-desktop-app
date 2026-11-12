//! "Check for updates" against GitHub Releases (PLAN §8c). Only tells the
//! user; never downloads or installs anything.

use std::time::Duration;

pub const REPO_URL: &str = "https://github.com/ankurk91/unspool-desktop-app";
pub const ISSUES_URL: &str = "https://github.com/ankurk91/unspool-desktop-app/issues/new";
const LATEST_URL: &str =
    "https://api.github.com/repos/ankurk91/unspool-desktop-app/releases/latest";
const TIMEOUT: Duration = Duration::from_secs(10);
/// Lines of release notes shown in the dialog.
const NOTE_LINES: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCheck {
    Newer {
        version: String,
        notes: String,
        url: String,
    },
    UpToDate {
        current: String,
    },
    /// The repository has no published release yet (404).
    NoReleases,
    /// Offline, timed out, rate-limited or an unexpected answer.
    Failed {
        reason: String,
    },
}

/// The running version (`CARGO_PKG_VERSION`).
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Checks the latest release. Blocking (up to 10 s): run off the UI thread.
/// `UNSPOOL_UPDATE_URL` overrides the endpoint for testing.
pub fn check() -> UpdateCheck {
    let url = std::env::var("UNSPOOL_UPDATE_URL").unwrap_or_else(|_| LATEST_URL.to_owned());
    check_url(&url, current_version())
}

pub fn check_url(url: &str, current: &str) -> UpdateCheck {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let response = agent
        .get(url)
        .header("User-Agent", &format!("Unspool/{current}"))
        .header("Accept", "application/vnd.github+json")
        .call();
    match response {
        Ok(mut r) => {
            let status = r.status().as_u16();
            let body = r.body_mut().read_to_string().unwrap_or_default();
            classify(status, &body, current)
        }
        Err(e) => UpdateCheck::Failed {
            reason: e.to_string(),
        },
    }
}

/// Turns GitHub's answer into a result (PLAN §8c table).
pub fn classify(status: u16, body: &str, current: &str) -> UpdateCheck {
    match status {
        200 => {}
        404 => return UpdateCheck::NoReleases,
        403 | 429 => {
            return UpdateCheck::Failed {
                reason: format!("GitHub rate limit reached (HTTP {status})"),
            };
        }
        other => {
            return UpdateCheck::Failed {
                reason: format!("GitHub answered HTTP {other}"),
            };
        }
    }
    let json: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return UpdateCheck::Failed {
                reason: format!("Unexpected reply: {e}"),
            };
        }
    };
    let tag = json["tag_name"].as_str().unwrap_or_default();
    let latest = tag.trim_start_matches(['v', 'V']);
    let (Ok(latest_v), Ok(current_v)) = (
        semver::Version::parse(latest),
        semver::Version::parse(current),
    ) else {
        return UpdateCheck::Failed {
            reason: format!("Can't compare versions \"{tag}\" and \"{current}\""),
        };
    };
    if latest_v > current_v {
        let notes = json["body"]
            .as_str()
            .unwrap_or_default()
            .lines()
            .take(NOTE_LINES)
            .collect::<Vec<_>>()
            .join("\n");
        UpdateCheck::Newer {
            version: latest_v.to_string(),
            notes,
            url: json["html_url"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{REPO_URL}/releases/latest")),
        }
    } else {
        UpdateCheck::UpToDate {
            current: current.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE: &str = r#"{"tag_name":"v0.2.0","html_url":"https://github.com/ankurk91/unspool-desktop-app/releases/tag/v0.2.0","body":"line1\nline2"}"#;

    #[test]
    fn newer_release() {
        assert_eq!(
            classify(200, RELEASE, "0.1.0"),
            UpdateCheck::Newer {
                version: "0.2.0".into(),
                notes: "line1\nline2".into(),
                url: "https://github.com/ankurk91/unspool-desktop-app/releases/tag/v0.2.0".into(),
            }
        );
    }

    #[test]
    fn same_or_older() {
        assert_eq!(
            classify(200, RELEASE, "0.2.0"),
            UpdateCheck::UpToDate {
                current: "0.2.0".into()
            }
        );
        assert!(matches!(
            classify(200, RELEASE, "0.3.1"),
            UpdateCheck::UpToDate { .. }
        ));
    }

    #[test]
    fn no_releases_and_errors() {
        assert_eq!(
            classify(404, r#"{"message":"Not Found"}"#, "0.1.0"),
            UpdateCheck::NoReleases
        );
        assert!(
            matches!(classify(403, "", "0.1.0"), UpdateCheck::Failed { reason } if reason.contains("rate limit"))
        );
        assert!(matches!(
            classify(429, "", "0.1.0"),
            UpdateCheck::Failed { .. }
        ));
        assert!(matches!(
            classify(500, "", "0.1.0"),
            UpdateCheck::Failed { .. }
        ));
        assert!(matches!(
            classify(200, "not json", "0.1.0"),
            UpdateCheck::Failed { .. }
        ));
        assert!(matches!(
            classify(200, r#"{"tag_name":"nightly"}"#, "0.1.0"),
            UpdateCheck::Failed { .. }
        ));
    }

    #[test]
    fn notes_are_trimmed_to_twenty_lines() {
        let body: String = (1..=30).map(|i| format!("l{i}\\n")).collect();
        let json = format!(r#"{{"tag_name":"1.0.0","body":"{body}"}}"#);
        let UpdateCheck::Newer { notes, url, .. } = classify(200, &json, "0.1.0") else {
            panic!("expected newer");
        };
        assert_eq!(notes.lines().count(), 20);
        assert!(url.ends_with("/releases/latest"));
    }

    /// Live check against GitHub. Run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "network"]
    fn live_repository_has_no_releases_yet() {
        assert_eq!(check_url(LATEST_URL, "0.1.0"), UpdateCheck::NoReleases);
    }

    #[test]
    fn offline_is_a_failure_not_a_hang() {
        // Port 9 (discard) is closed on a normal machine: connection refused.
        let t = std::time::Instant::now();
        let r = check_url("http://127.0.0.1:9/releases/latest", "0.1.0");
        assert!(matches!(r, UpdateCheck::Failed { .. }), "{r:?}");
        assert!(t.elapsed() < TIMEOUT + Duration::from_secs(1));
    }
}
