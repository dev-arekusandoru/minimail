//! Update checks against the GitHub release of this build's channel. No GPUI.
//!
//! `scripts/package.sh` bakes the channel, build number and version into packaged builds and
//! writes a `latest.json` manifest next to the DMG; dev builds have no channel and never check.
use crate::clock::{DAY, Timestamp};
use serde::Deserialize;
use std::time::Duration;

const REPO: &str = "https://github.com/dev-arekusandoru/minimail";

/// How long after a check the next automatic one is due.
pub const CHECK_EVERY: Timestamp = DAY;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// The rolling `nightly` prerelease.
    Nightly,
    /// The latest non-prerelease GitHub release.
    Release,
}

impl Channel {
    pub fn label(self) -> &'static str {
        match self {
            Channel::Nightly => "nightly",
            Channel::Release => "release",
        }
    }

    /// Where the channel's assets download from; `latest/download` skips prereleases.
    fn download_base(self) -> String {
        match self {
            Channel::Nightly => format!("{REPO}/releases/download/nightly"),
            Channel::Release => format!("{REPO}/releases/latest/download"),
        }
    }

    pub fn manifest_url(self) -> String {
        format!("{}/latest.json", self.download_base())
    }

    pub fn release_page(self) -> String {
        match self {
            Channel::Nightly => format!("{REPO}/releases/tag/nightly"),
            Channel::Release => format!("{REPO}/releases/latest"),
        }
    }
}

/// The running build, as baked in by `scripts/package.sh`.
#[derive(Clone, Copy, Debug)]
pub struct Build {
    pub channel: Channel,
    /// Increases with every build of a channel (CI's run number).
    pub number: u64,
    pub version: &'static str,
}

/// This build, or `None` for dev builds (no channel baked in), which never update.
pub fn current() -> Option<Build> {
    let channel = match option_env!("MAIL_CLASSIFIER_CHANNEL")? {
        "nightly" => Channel::Nightly,
        "release" => Channel::Release,
        _ => return None,
    };
    Some(Build {
        channel,
        number: option_env!("MAIL_CLASSIFIER_BUILD")?.parse().ok()?,
        version: option_env!("MAIL_CLASSIFIER_VERSION")?,
    })
}

/// `latest.json`, uploaded next to the DMG by the release workflow.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub build: u64,
    /// The DMG's file name, relative to the channel's download base.
    pub dmg: String,
    /// Lowercase hex SHA-256 of the DMG.
    pub sha256: String,
}

impl Manifest {
    pub fn is_newer_than(&self, build: &Build) -> bool {
        self.build > build.number
    }
}

/// Whether an automatic check is due, given when the last one ran.
pub fn check_due(last: Option<Timestamp>, now: Timestamp) -> bool {
    last.is_none_or(|last| now - last >= CHECK_EVERY)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(60))).build().into()
}

/// Fetch the channel's manifest. Blocking: run it off the UI thread.
pub fn fetch_manifest(channel: Channel) -> Result<Manifest, String> {
    agent()
        .get(&channel.manifest_url())
        .call()
        .and_then(|mut res| res.body_mut().read_json())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{Build, CHECK_EVERY, Channel, Manifest, check_due};

    #[test]
    fn checks_are_due_first_and_then_once_a_day() {
        assert!(check_due(None, 0));
        assert!(!check_due(Some(100), 100 + CHECK_EVERY - 1));
        assert!(check_due(Some(100), 100 + CHECK_EVERY));
    }

    #[test]
    fn only_a_higher_build_number_is_newer() {
        let manifest: Manifest = serde_json::from_str(
            r#"{"version":"0.1.0-nightly.20261021","build":42,"dmg":"Minimail.dmg","sha256":"ab"}"#,
        )
        .unwrap();
        let build = |number| Build { channel: Channel::Nightly, number, version: "x" };
        assert!(manifest.is_newer_than(&build(41)));
        assert!(!manifest.is_newer_than(&build(42)));
        assert!(!manifest.is_newer_than(&build(43)));
    }
}
