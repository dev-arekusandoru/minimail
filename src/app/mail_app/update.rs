use super::*;
use crate::update::{self, Build, Manifest};

#[derive(Default)]
pub(super) enum UpdateState {
    #[default]
    Idle,
    Checking,
    Available(Manifest),
}

impl MailApp {
    /// Run the daily automatic check when it is on and due. Called from `tick`.
    pub(super) fn auto_check_update(&mut self, now: Timestamp, cx: &mut Context<Self>) {
        if self.auto_update_check
            && matches!(self.update_state, UpdateState::Idle)
            && update::check_due(self.update_checked_at, now)
        {
            self.check_update(false, now, cx);
        }
    }

    /// The "Check for updates" command.
    pub(super) fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = match &self.update_state {
            UpdateState::Checking => "Already checking for updates".to_owned(),
            UpdateState::Available(manifest) => {
                let text = format!("Minimail {} is available", manifest.version);
                return self.push_toast(text, Some(UPDATE_ACTION), window, cx);
            }
            UpdateState::Idle if update::current().is_none() => "Development builds don't update".to_owned(),
            UpdateState::Idle => {
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
        self.update_state = UpdateState::Checking;
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
                let text = format!("Minimail {} is available", manifest.version);
                self.pending_toast = Some((text, Some(UPDATE_ACTION)));
                self.update_state = UpdateState::Available(manifest);
            }
            Ok(_) if manual => self.pending_toast = Some((format!("Minimail {} is up to date", build.version), None)),
            Ok(_) => {}
            Err(e) if manual => self.pending_toast = Some((format!("Update check failed: {e}"), None)),
            Err(e) => eprintln!("update check failed: {e}"),
        }
        cx.notify();
    }

    /// The toast's "Update" button: open the release page to download the new build.
    pub(super) fn install_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match (&self.update_state, update::current()) {
            (UpdateState::Available(_), Some(build)) => cx.open_url(&build.channel.release_page()),
            _ => self.show_toast("No update available".to_owned(), window, cx),
        }
    }
}

const UPDATE_ACTION: ToastAction = ToastAction {
    id: "toast-update",
    label: "Update",
    key: None,
    action: || Box::new(InstallUpdate),
    autohide: false,
};
