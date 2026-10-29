//! Update checks against the GitHub release of this build's channel. No GPUI.
//!
//! `scripts/package.sh` bakes the channel, build number and version into packaged builds and
//! writes a `latest.json` manifest next to the DMG; dev builds have no channel and never check.
use crate::clock::{DAY, Timestamp};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const REPO: &str = "https://github.com/dev-arekusandoru/minimail";
/// The app bundle's name inside the DMG.
const APP_BUNDLE: &str = "Minimail.app";
/// Refuse DMGs past this size.
const MAX_DMG_BYTES: u64 = 500 << 20;

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    /// The last check failed.
    Failed,
    Available(Manifest),
    /// Downloading and verifying this version.
    Downloading(String),
    /// The new build sits verified beside the app, swapped in on restart.
    Ready { staged: PathBuf, version: String },
}

/// How the status line's lead text is coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Muted,
    Success,
    Accent,
    Error,
}

/// The Settings → Updates row's button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateButton {
    Check,
    Checking,
    Retry,
    Update,
    Downloading,
    Restart,
    /// Dev builds: "Check now", disabled.
    Unavailable,
}

impl UpdateButton {
    pub fn label(self) -> &'static str {
        match self {
            UpdateButton::Check | UpdateButton::Unavailable => "Check now",
            UpdateButton::Checking => "Checking…",
            UpdateButton::Retry => "Try again",
            UpdateButton::Update => "Update",
            UpdateButton::Downloading => "Downloading…",
            UpdateButton::Restart => "Restart",
        }
    }

    /// Shows a spinner and ignores clicks.
    pub fn busy(self) -> bool {
        matches!(self, UpdateButton::Checking | UpdateButton::Downloading)
    }

    /// Moves the update forward (download or restart) rather than checking.
    pub fn installs(self) -> bool {
        matches!(self, UpdateButton::Update | UpdateButton::Restart)
    }
}

/// The Settings → Updates status row (`mockups/updates-default.html`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub title: String,
    pub lead: String,
    pub tone: Tone,
    /// Muted text after the lead, joined with " · ".
    pub detail: Option<String>,
    pub button: UpdateButton,
}

/// The status row for `build` (`None` = dev build) in `state`, last checked at `checked_at`.
pub fn status(build: Option<&Build>, state: &UpdateState, checked_at: Option<Timestamp>, now: Timestamp) -> Status {
    let title = format!("Minimail {}", build.map_or(env!("CARGO_PKG_VERSION"), |b| b.version));
    let checked = |prefix: &str| checked_at.map(|at| format!("{prefix}{}", crate::sync_status::ago(now - at)));
    let (lead, tone, detail, button) = match state {
        _ if build.is_none() => ("Development builds don’t update".into(), Tone::Muted, None, UpdateButton::Unavailable),
        UpdateState::Idle if checked_at.is_none() => ("Not checked yet".into(), Tone::Muted, None, UpdateButton::Check),
        UpdateState::Idle => ("Up to date".into(), Tone::Success, checked("checked "), UpdateButton::Check),
        UpdateState::Checking => ("Checking…".into(), Tone::Muted, None, UpdateButton::Checking),
        UpdateState::Failed => ("Check failed".into(), Tone::Error, checked(""), UpdateButton::Retry),
        UpdateState::Available(m) => (format!("{} is available", m.version), Tone::Accent, None, UpdateButton::Update),
        UpdateState::Downloading(v) => (format!("Downloading {v}…"), Tone::Muted, None, UpdateButton::Downloading),
        UpdateState::Ready { version, .. } => (format!("{version} is ready"), Tone::Accent, None, UpdateButton::Restart),
    };
    Status { title, lead, tone, detail, button }
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

/// The running `.app` bundle, or `None` when the binary isn't inside one.
pub fn running_app() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.ancestors().nth(3)?; // <app>/Contents/MacOS/<exe>
    (app.extension()? == "app").then(|| app.to_path_buf())
}

/// Download the manifest's DMG into `dir`, refusing it unless its SHA-256 matches. Blocking.
pub fn download(channel: Channel, manifest: &Manifest, dir: &Path) -> Result<PathBuf, String> {
    let url = format!("{}/{}", channel.download_base(), manifest.dmg);
    let bytes = agent()
        .get(&url)
        .call()
        .and_then(|mut res| res.body_mut().with_config().limit(MAX_DMG_BYTES).read_to_vec())
        .map_err(|e| e.to_string())?;
    if format!("{:x}", Sha256::digest(&bytes)) != manifest.sha256.to_ascii_lowercase() {
        return Err("the download doesn't match its checksum".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join("update.dmg");
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Copy the DMG's app next to `target` once it passes `target`'s own code signing requirement
/// (same identifier and team) and Gatekeeper. Returns the staged copy. Blocking.
pub fn stage(dmg: &Path, target: &Path) -> Result<PathBuf, String> {
    let requirement = designated_requirement(target)?;
    let mount = dmg.with_extension("mount");
    let attach = ["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"];
    run(Command::new("hdiutil").args(attach).arg(&mount).arg(dmg))?;
    let staged = (|| {
        let app = mount.join(APP_BUNDLE);
        run(Command::new("codesign").args(["--verify", "--deep", "--strict", "-R"]).arg(format!("={requirement}")).arg(&app))
            .map_err(|e| format!("the update isn't signed like this app ({e})"))?;
        run(Command::new("spctl").args(["--assess", "--type", "execute"]).arg(&app))
            .map_err(|e| format!("Gatekeeper rejected the update ({e})"))?;
        let staged = target.with_file_name(".Minimail-update.app");
        let _ = std::fs::remove_dir_all(&staged);
        run(Command::new("ditto").arg(&app).arg(&staged))?;
        Ok(staged)
    })();
    let _ = run(Command::new("hdiutil").args(["detach", "-quiet"]).arg(&mount));
    staged
}

/// `target`'s designated requirement, e.g. `identifier "…" and anchor apple generic and …`.
/// Ad-hoc builds only have an implicit (commented-out) requirement, so they never update.
fn designated_requirement(target: &Path) -> Result<String, String> {
    let out = Command::new("codesign")
        .args(["-d", "-r-"])
        .arg(target)
        .output()
        .map_err(|e| e.to_string())?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("designated => "))
        .map(str::to_owned)
        .ok_or_else(|| "this build isn't Developer ID signed, so it can't verify updates".into())
}

fn run(command: &mut Command) -> Result<(), String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let out = command.output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!("{program}: {}", String::from_utf8_lossy(&out.stderr).trim()))
}

/// Start a detached shell that waits for process `pid` to exit, swaps `staged` in for `target`
/// (restoring `target` if the swap fails) and relaunches it. A running app can't replace itself.
pub fn install(staged: &Path, target: &Path, pid: u32) -> std::io::Result<()> {
    Command::new("/bin/sh")
        .args(["-c", SWAP_SCRIPT, "sh", &pid.to_string()])
        .arg(staged)
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

const SWAP_SCRIPT: &str = r#"
while kill -0 "$1" 2>/dev/null; do sleep 0.2; done
old="$3.old"
rm -rf "$old"
if mv "$3" "$old"; then
    if mv "$2" "$3"; then rm -rf "$old"; else mv "$old" "$3"; fi
fi
open "$3"
"#;

#[cfg(test)]
mod tests {
    use super::{Build, CHECK_EVERY, Channel, Manifest, Tone, UpdateButton, UpdateState, check_due, status};
    use crate::clock::HOUR;

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

    #[test]
    fn status_row_follows_the_state_and_last_check() {
        let build = Build { channel: Channel::Nightly, number: 1, version: "0.1.0-nightly.20261020" };
        let now = 10 * HOUR;
        let s = status(Some(&build), &UpdateState::Idle, None, now);
        assert_eq!((s.title.as_str(), s.lead.as_str(), s.button), ("Minimail 0.1.0-nightly.20261020", "Not checked yet", UpdateButton::Check));
        let s = status(Some(&build), &UpdateState::Idle, Some(now - 2 * HOUR), now);
        assert_eq!((s.lead.as_str(), s.tone, s.detail.as_deref()), ("Up to date", Tone::Success, Some("checked 2 h ago")));
        let s = status(Some(&build), &UpdateState::Failed, Some(now - 2 * HOUR), now);
        assert_eq!((s.tone, s.detail.as_deref(), s.button), (Tone::Error, Some("2 h ago"), UpdateButton::Retry));
        // A dev build never offers a check, whatever the state says.
        let s = status(None, &UpdateState::Checking, None, now);
        assert_eq!((s.lead.as_str(), s.button), ("Development builds don’t update", UpdateButton::Unavailable));
    }
}
