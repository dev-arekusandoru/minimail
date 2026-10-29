use super::*;
use crate::update::{self, Build, Manifest, UpdateState};
use std::path::PathBuf;

impl MailApp {
    /// Run the daily automatic check when it is on and due. Called from `tick`.
    pub(super) fn auto_check_update(&mut self, now: Timestamp, cx: &mut Context<Self>) {
        if self.auto_update_check
            && matches!(self.update_state, UpdateState::Idle | UpdateState::Failed)
            && update::check_due(self.update_checked_at, now)
        {
            self.check_update(false, now, cx);
        }
    }

    /// The Settings → Updates status row as of now.
    pub(super) fn update_status(&self) -> update::Status {
        update::status(update::current().as_ref(), &self.update_state, self.update_checked_at, self.now())
    }

    /// Show the update status on an open Settings panel.
    pub(super) fn sync_settings_update(&self, cx: &mut Context<Self>) {
        if let Some(panel) = &self.settings {
            let status = self.update_status();
            panel.update(cx, |panel, cx| panel.show_update_status(status, cx));
        }
    }

    /// The Settings → Updates button: check, or move the found update forward.
    pub(super) fn settings_update_button(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.update_status().button {
            button if button.installs() => self.install_update(window, cx),
            update::UpdateButton::Check | update::UpdateButton::Retry => self.check_update(false, self.now(), cx),
            _ => {}
        }
    }

    /// The "Check for updates" command.
    pub(super) fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = match &self.update_state {
            UpdateState::Checking => "Already checking for updates".to_owned(),
            UpdateState::Downloading(_) => "Downloading the update…".to_owned(),
            UpdateState::Available(manifest) => {
                let text = available_text(manifest);
                return self.push_toast(text, Some(UPDATE_ACTION), window, cx);
            }
            UpdateState::Ready { version, .. } => {
                let text = ready_text(version);
                return self.push_toast(text, Some(RESTART_ACTION), window, cx);
            }
            UpdateState::Idle | UpdateState::Failed if update::current().is_none() => {
                "Development builds don't update".to_owned()
            }
            UpdateState::Idle | UpdateState::Failed => {
                self.check_update(true, self.now(), cx);
                "Checking for updates…".to_owned()
            }
        };
        self.show_toast(text, window, cx);
    }

    fn check_update(&mut self, manual: bool, now: Timestamp, cx: &mut Context<Self>) {
        let Some(build) = update::current() else {
            return;
        };
        // Stamped up front so a failed automatic check waits a day instead of retrying each tick.
        self.update_checked_at = Some(now);
        self.save_setting(&crate::app_settings::UPDATE_CHECKED_AT, Some(now));
        self.update_state = UpdateState::Checking;
        self.sync_settings_update(cx);
        let task = cx.background_spawn(async move { update::fetch_manifest(build.channel) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| this.finish_update_check(build, result, manual, cx)).ok();
        })
        .detach();
    }

    fn finish_update_check(
        &mut self,
        build: Build,
        result: Result<Manifest, String>,
        manual: bool,
        cx: &mut Context<Self>,
    ) {
        self.update_state = UpdateState::Idle;
        match result {
            Ok(manifest) if manifest.is_newer_than(&build) => {
                self.pending_toast = Some((available_text(&manifest), Some(UPDATE_ACTION)));
                self.update_state = UpdateState::Available(manifest);
            }
            Ok(_) if manual => self.pending_toast = Some((format!("Minimail {} is up to date", build.version), None)),
            Ok(_) => {}
            Err(e) => {
                self.update_state = UpdateState::Failed;
                if manual {
                    self.pending_toast = Some((format!("Update check failed: {e}"), None));
                } else {
                    eprintln!("update check failed: {e}");
                }
            }
        }
        self.sync_settings_update(cx);
        cx.notify();
    }

    /// The toasts' "Update" and "Restart" buttons: download and verify the new build, then swap
    /// it in and relaunch.
    pub(super) fn install_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(build), Some(target)) = (update::current(), update::running_app()) else {
            return self.show_toast("No update available".to_owned(), window, cx);
        };
        match std::mem::take(&mut self.update_state) {
            UpdateState::Available(manifest) => {
                self.show_toast(format!("Downloading Minimail {}…", manifest.version), window, cx);
                self.update_state = UpdateState::Downloading(manifest.version.clone());
                let task = cx.background_spawn(async move {
                    let dir = std::env::temp_dir().join("minimail-update");
                    let staged = update::download(build.channel, &manifest, &dir)
                        .and_then(|dmg| update::stage(&dmg, &target));
                    (manifest, staged)
                });
                cx.spawn(async move |this, cx| {
                    let (manifest, staged) = task.await;
                    this.update(cx, |this, cx| this.finish_download(manifest, staged, cx)).ok();
                })
                .detach();
            }
            UpdateState::Ready { staged, version } => match update::install(&staged, &target, std::process::id()) {
                Ok(()) => window.dispatch_action(Box::new(Quit), cx),
                Err(e) => {
                    self.update_state = UpdateState::Ready { staged, version };
                    self.show_toast(format!("Update failed: {e}"), window, cx);
                }
            },
            state => {
                let text = match state {
                    UpdateState::Downloading(_) => "Downloading the update…",
                    _ => "No update available",
                };
                self.update_state = state;
                self.show_toast(text.to_owned(), window, cx);
            }
        }
        self.sync_settings_update(cx);
    }

    fn finish_download(&mut self, manifest: Manifest, staged: Result<PathBuf, String>, cx: &mut Context<Self>) {
        match staged {
            Ok(staged) => {
                self.pending_toast = Some((ready_text(&manifest.version), Some(RESTART_ACTION)));
                self.update_state = UpdateState::Ready { staged, version: manifest.version };
            }
            // Can't install in place (say, /Applications isn't writable): fall back to the page.
            Err(e) => {
                self.pending_toast = Some((format!("Update failed: {e} · opening the download page"), None));
                if let Some(build) = update::current() {
                    cx.open_url(&build.channel.release_page());
                }
                self.update_state = UpdateState::Available(manifest);
            }
        }
        self.sync_settings_update(cx);
        cx.notify();
    }
}

fn available_text(manifest: &Manifest) -> String {
    format!("Minimail {} is available", manifest.version)
}

fn ready_text(version: &str) -> String {
    format!("Minimail {version} is ready · restart to update")
}

const UPDATE_ACTION: ToastAction = ToastAction {
    id: "toast-update",
    label: "Update",
    key: None,
    action: || Box::new(InstallUpdate),
    autohide: false,
};

const RESTART_ACTION: ToastAction = ToastAction {
    id: "toast-restart",
    label: "Restart",
    key: None,
    action: || Box::new(InstallUpdate),
    autohide: false,
};
