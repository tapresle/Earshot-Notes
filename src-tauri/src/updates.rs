//! Checking GitHub for a newer release of Earshot Notes. The app only checks
//! when you press **Check for updates**, and installs nothing: it shows the
//! release notes and opens the release page in the browser.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Off until the repository is public: GitHub's releases API answers 404 for
/// a private repository without signing in, and the app never carries a token.
/// TODO(#62): turn this on when tapresle/Earshot is public, then remove it.
pub const ENABLED: bool = false;

const REPOSITORY: &str = "https://github.com/tapresle/Earshot";
const LATEST_RELEASE: &str = "https://api.github.com/repos/tapresle/Earshot/releases/latest";
/// Release notes are a few kilobytes; anything far larger isn't a release.
const MAX_RESPONSE_BYTES: u64 = 1 << 20;

/// A plain `MAJOR.MINOR.PATCH` version, as Earshot's releases use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u64, u64, u64);

impl FromStr for Version {
    type Err = UpdateError;

    /// Accepts `1.2.3` or `v1.2.3`, and nothing else, so a version can safely
    /// become part of a URL.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let unreadable = || UpdateError::Unreadable(format!("{text:?} isn't a version"));
        let digits = text.strip_prefix('v').unwrap_or(text);
        let parts = digits
            .split('.')
            .map(|part| {
                if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(unreadable());
                }
                part.parse::<u64>().map_err(|_| unreadable())
            })
            .collect::<Result<Vec<_>, _>>()?;
        match parts[..] {
            [major, minor, patch] => Ok(Version(major, minor, patch)),
            _ => Err(unreadable()),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// What a check found.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub current: String,
    pub latest: String,
    /// Whether `latest` is newer than the running version.
    pub newer: bool,
    /// The latest release's notes, as Markdown without links or the download section.
    pub notes: String,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum UpdateError {
    #[error("couldn't reach GitHub, check your internet connection: {0}")]
    Network(String),
    #[error("GitHub is limiting requests from this network, try again later")]
    RateLimited,
    #[error("GitHub has no published release of Earshot Notes")]
    NoRelease,
    #[error("GitHub answered with HTTP {0}")]
    Status(u16),
    #[error("GitHub's answer isn't a release this version can read: {0}")]
    Unreadable(String),
}

/// A finished HTTP response.
pub struct Response {
    pub status: u16,
    pub body: String,
}

/// Fetches a URL. A trait so tests can answer with recorded responses.
pub trait Fetch {
    fn get(&self, url: &str) -> Result<Response, String>;
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    body: Option<String>,
}

/// Asks GitHub for the latest release and compares it with `current`.
pub fn check(fetch: &dyn Fetch, current: &str) -> Result<UpdateCheck, UpdateError> {
    let running: Version = current.parse()?;
    let response = fetch.get(LATEST_RELEASE).map_err(UpdateError::Network)?;
    match response.status {
        200 => {}
        404 => return Err(UpdateError::NoRelease),
        // GitHub answers 403 when the hourly limit for unauthenticated requests runs out.
        403 | 429 => return Err(UpdateError::RateLimited),
        status => return Err(UpdateError::Status(status)),
    }
    let release: Release = serde_json::from_str(&response.body)
        .map_err(|error| UpdateError::Unreadable(error.to_string()))?;
    let latest: Version = release.tag_name.parse()?;
    Ok(UpdateCheck {
        current: running.to_string(),
        latest: latest.to_string(),
        newer: latest > running,
        notes: display_notes(release.body.as_deref().unwrap_or_default()),
    })
}

/// The release page for a version. The URL is built here, never taken from GitHub's answer.
pub fn release_page(version: &str) -> Result<String, UpdateError> {
    let version: Version = version.parse()?;
    Ok(format!("{REPOSITORY}/releases/tag/v{version}"))
}

/// Prepares release notes for the app's Markdown renderer, which has no links:
/// each link becomes its text, and the download section, which lists a file
/// and its checksum, is left out because the release page has it. Headings
/// move down a level to sit under the Updates heading in Settings.
pub fn display_notes(body: &str) -> String {
    let mut kept = Vec::new();
    let mut skipping = false;
    for line in body.lines() {
        if line.starts_with("## ") {
            skipping = line.trim() == "## Download";
        }
        if skipping {
            continue;
        }
        let line = without_links(line);
        kept.push(if line.starts_with('#') {
            format!("#{line}")
        } else {
            line
        });
    }
    kept.join("\n").trim().to_owned()
}

/// Replaces each `[text](target)` with `text`.
fn without_links(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let link = after.find("](").and_then(|close| {
            let target = &after[close + 2..];
            target.find(')').map(|end| (close, close + 2 + end + 1))
        });
        match link {
            Some((close, consumed)) => {
                out.push_str(&rest[..open]);
                out.push_str(&after[..close]);
                rest = &after[consumed..];
            }
            None => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Fetches over HTTPS with ureq.
pub struct HttpFetch {
    agent: ureq::Agent,
}

impl Default for HttpFetch {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Fetch for HttpFetch {
    fn get(&self, url: &str) -> Result<Response, String> {
        let response = self
            .agent
            .get(url)
            // GitHub refuses requests without a user agent.
            .header(
                "User-Agent",
                concat!("Earshot-Notes/", env!("CARGO_PKG_VERSION")),
            )
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .call()
            .map_err(|error| error.to_string())?;
        let status = response.status().as_u16();
        let body = response
            .into_body()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_string()
            .map_err(|error| error.to_string())?;
        Ok(Response { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Answers with one recorded response, and records the URLs asked for.
    struct Recorded {
        response: Result<(u16, &'static str), &'static str>,
        asked: RefCell<Vec<String>>,
    }

    impl Recorded {
        fn ok(status: u16, body: &'static str) -> Self {
            Self {
                response: Ok((status, body)),
                asked: RefCell::default(),
            }
        }
    }

    impl Fetch for Recorded {
        fn get(&self, url: &str) -> Result<Response, String> {
            self.asked.borrow_mut().push(url.to_owned());
            self.response
                .map(|(status, body)| Response {
                    status,
                    body: body.to_owned(),
                })
                .map_err(str::to_owned)
        }
    }

    // Trimmed from the shape of GitHub's response for a release; the notes are synthetic.
    const RELEASE_0_2_0: &str = r#"{
        "tag_name": "v0.2.0",
        "name": "Earshot Notes 0.2.0",
        "draft": false,
        "prerelease": false,
        "html_url": "https://example.com/not-used",
        "published_at": "2026-11-02T18:00:00Z",
        "body": "Released November 2, 2026.\n\n## Improved\n\n- **Faster summaries.** ([#90](https://github.com/tapresle/Earshot/issues/90))\n\n## Download\n\n`Earshot-Notes_0.2.0_aarch64.dmg`, SHA-256: `abc123`\n"
    }"#;

    #[test]
    fn finds_a_newer_release_and_its_notes() {
        let github = Recorded::ok(200, RELEASE_0_2_0);

        let found = check(&github, "0.1.6").unwrap();

        assert_eq!(*github.asked.borrow(), vec![LATEST_RELEASE.to_owned()]);
        assert_eq!(
            found,
            UpdateCheck {
                current: "0.1.6".into(),
                latest: "0.2.0".into(),
                newer: true,
                notes:
                    "Released November 2, 2026.\n\n### Improved\n\n- **Faster summaries.** (#90)"
                        .into(),
            }
        );
    }

    #[test]
    fn reports_no_update_for_the_same_or_an_older_release() {
        let github = Recorded::ok(200, RELEASE_0_2_0);

        assert!(!check(&github, "0.2.0").unwrap().newer);
        assert!(!check(&github, "0.10.0").unwrap().newer);
    }

    #[test]
    fn explains_each_failure() {
        let offline = Recorded {
            response: Err("dns lookup failed"),
            asked: RefCell::default(),
        };
        assert_eq!(
            check(&offline, "0.1.6"),
            Err(UpdateError::Network("dns lookup failed".into()))
        );
        assert_eq!(
            check(&Recorded::ok(404, r#"{"message":"Not Found"}"#), "0.1.6"),
            Err(UpdateError::NoRelease)
        );
        assert_eq!(
            check(
                &Recorded::ok(403, r#"{"message":"API rate limit exceeded"}"#),
                "0.1.6"
            ),
            Err(UpdateError::RateLimited)
        );
        assert_eq!(
            check(&Recorded::ok(502, ""), "0.1.6"),
            Err(UpdateError::Status(502))
        );
        assert!(matches!(
            check(&Recorded::ok(200, "<html>"), "0.1.6"),
            Err(UpdateError::Unreadable(_))
        ));
        assert!(matches!(
            check(&Recorded::ok(200, r#"{"tag_name":"nightly"}"#), "0.1.6"),
            Err(UpdateError::Unreadable(_))
        ));
    }

    #[test]
    fn reads_only_plain_versions() {
        assert_eq!("0.1.6".parse::<Version>(), Ok(Version(0, 1, 6)));
        assert_eq!("v12.0.3".parse::<Version>(), Ok(Version(12, 0, 3)));
        for bad in [
            "",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "1..3",
            "v1.2.3-beta",
            "1.2.3/../x",
            " 1.2.3",
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad:?} should be refused");
        }
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
    }

    #[test]
    fn builds_the_release_page_from_the_version_only() {
        assert_eq!(
            release_page("0.2.0").unwrap(),
            "https://github.com/tapresle/Earshot/releases/tag/v0.2.0"
        );
        assert!(release_page("0.2.0/../../evil").is_err());
    }

    #[test]
    fn turns_links_into_their_text() {
        assert_eq!(
            without_links("See [#41](https://x/41) and [the notes](https://x/n)."),
            "See #41 and the notes."
        );
        assert_eq!(
            without_links("[not a link] and [half](open"),
            "[not a link] and [half](open"
        );
    }

    /// Needs the network: `cargo test --lib -- --ignored reads_a_real_github_release`.
    /// Earshot's own repository is private, so this reads a public one.
    #[test]
    #[ignore]
    fn reads_a_real_github_release() {
        let response = HttpFetch::default()
            .get("https://api.github.com/repos/tauri-apps/tauri/releases/latest")
            .unwrap();

        assert_eq!(response.status, 200);
        let release: Release = serde_json::from_str(&response.body).unwrap();
        assert!(!release.tag_name.is_empty());
    }

    #[test]
    fn leaves_out_only_the_download_section_and_lowers_headings() {
        let notes = "Intro\n\n## Download\n\nfile, SHA-256: `x`\n\n## License\n\nGPL";

        assert_eq!(display_notes(notes), "Intro\n\n### License\n\nGPL");
    }
}
