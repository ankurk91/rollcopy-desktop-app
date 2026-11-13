//! Details for bug reports: shown in About ("Copy debug info") and filled
//! into the GitHub issue form by *Report an Issue*. Nothing personal: no
//! device name, serial, UDID, paths or file names.

use crate::update;

/// Longest issue link built. Browsers and GitHub cope with more, but some
/// launchers and proxies cut URLs near 2,000 characters.
const MAX_URL: usize = 2000;
/// Longest single value put in the link.
const MAX_VALUE: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugInfo {
    pub version: String,
    pub os: String,
    pub desktop: String,
    pub usbmuxd: String,
    pub ffmpeg: String,
    /// Model and iOS version, or "not connected".
    pub phone: String,
}

impl DebugInfo {
    /// Collects the details. Runs `usbmuxd --version` and `ffmpeg -version`,
    /// so call it off the UI thread when it matters.
    pub fn collect(phone: Option<String>) -> Self {
        let os = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                    .map(|v| v.trim_matches('"').to_owned())
            })
            .unwrap_or_else(|| "unknown".into());
        let desktop = format!(
            "{} ({})",
            std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into()),
            std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into())
        );
        Self {
            version: format!("{} ({})", update::current_version(), crate::about::GIT_HASH),
            os,
            desktop,
            usbmuxd: tool_version("usbmuxd", "--version", |l| l.to_owned()),
            ffmpeg: tool_version("ffmpeg", "-version", |l| {
                // "ffmpeg version 6.1.1-3ubuntu5 Copyright …" → "6.1.1-3ubuntu5"
                l.split_whitespace().nth(2).unwrap_or(l).to_owned()
            }),
            phone: phone.unwrap_or_else(|| "not connected".into()),
        }
    }

    pub fn text(&self) -> String {
        format!(
            "Rollcopy {}\nOS: {}\nDesktop: {}\nusbmuxd: {}\nffmpeg: {}\nPhone: {}\nLog: {}",
            self.version,
            self.os,
            self.desktop,
            self.usbmuxd,
            self.ffmpeg,
            self.phone,
            crate::paths::app_dirs().log().display()
        )
    }

    /// A "new issue" link that opens the bug-report form with these details
    /// filled in (GitHub fills issue-form fields from query parameters named
    /// after their ids). Kept under [`MAX_URL`] characters.
    pub fn issue_url(&self) -> String {
        let mut url = format!("{}?template=bug_report.yml", update::ISSUES_URL);
        for (id, value) in [
            ("version", &self.version),
            ("os", &self.os),
            ("desktop", &self.desktop),
            ("phone", &self.phone),
            ("usbmuxd", &self.usbmuxd),
            ("ffmpeg", &self.ffmpeg),
        ] {
            let value: String = value.chars().take(MAX_VALUE).collect();
            let param = format!("&{id}={}", encode(&value));
            if url.len() + param.len() > MAX_URL {
                break;
            }
            url.push_str(&param);
        }
        url
    }
}

fn tool_version(cmd: &str, arg: &str, pick: impl Fn(&str) -> String) -> String {
    std::process::Command::new(cmd)
        .arg(arg)
        .output()
        .ok()
        .and_then(|o| {
            let text = String::from_utf8_lossy(&o.stdout).to_string()
                + &String::from_utf8_lossy(&o.stderr);
            let line = text.lines().next()?.trim().to_owned();
            (!line.is_empty()).then(|| pick(&line))
        })
        .unwrap_or_else(|| "not found".into())
}

/// Percent-encodes a query value (RFC 3986 unreserved characters kept).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{DebugInfo, MAX_URL, encode};

    fn info() -> DebugInfo {
        DebugInfo {
            version: "0.1.0 (abc1234)".into(),
            os: "Ubuntu 24.04.1 LTS".into(),
            desktop: "GNOME (wayland)".into(),
            usbmuxd: "usbmuxd 1.1.1".into(),
            ffmpeg: "6.1.1-3ubuntu5".into(),
            phone: "iPhone 15 on iOS 26.1 (23B85)".into(),
        }
    }

    #[test]
    fn link_fills_the_form() {
        let url = info().issue_url();
        assert!(url.starts_with(
            "https://github.com/ankurk91/rollcopy-desktop-app/issues/new?template=bug_report.yml&"
        ));
        assert!(url.contains("&version=0.1.0%20%28abc1234%29"));
        assert!(url.contains("&os=Ubuntu%2024.04.1%20LTS"));
        assert!(url.contains("&phone=iPhone%2015%20on%20iOS%2026.1%20%2823B85%29"));
    }

    #[test]
    fn link_stays_short() {
        let mut i = info();
        let long = "é".repeat(5000);
        i.os = long.clone();
        i.desktop = long.clone();
        i.phone = long.clone();
        i.usbmuxd = long.clone();
        i.ffmpeg = long;
        let url = i.issue_url();
        assert!(url.len() <= MAX_URL, "{}", url.len());
        assert!(url.contains("&version="));
    }

    #[test]
    fn encoding() {
        assert_eq!(encode("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
    }
}
