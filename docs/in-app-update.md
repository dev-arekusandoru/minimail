# In-app update

Packaged builds check their release channel for a newer build, download and verify it,
and swap it in on restart.

## Channels

`scripts/package.sh` bakes `MAIL_CLASSIFIER_CHANNEL`, `_BUILD` and `_VERSION` into the
binary (`update::current()`); dev builds have none and never check. Each channel reads a
`latest.json` (`version`, `build`, `dmg`, `sha256`) that `package.sh` writes next to the DMG:

| Channel | Manifest |
|---|---|
| nightly | `releases/download/nightly/latest.json` (the rolling prerelease) |
| release | `releases/latest/download/latest.json` (latest non-prerelease) |

A manifest is newer when its `build` is higher, so `BUILD_NUMBER` must increase within a
channel; CI passes its run number. A release workflow only has to run `package.sh` with
`CHANNEL=release` and upload the DMG and `dist/latest.json` to a normal (non-pre) release.

## Flow

- **Check**: on launch, then every 24 h while the app runs (Settings → Updates turns this
  off), or via the palette's "Check for updates". The last check time persists
  (`UPDATE_CHECKED_AT`), so a relaunch doesn't re-check early. A found build shows a toast
  with Update.
- **Settings → Updates** (`mockups/updates-default.html`): the version, the status
  (`update::status`: up to date · checked 2 h ago, available, ready, failed…) and one button
  that checks, updates or restarts, then the auto-check switch.
- **Update** (`update::download` + `update::stage`): download the DMG, check its sha256,
  mount it, require the app inside to meet the running app's own designated requirement
  (same bundle id and Developer ID team) and pass `spctl`, then `ditto` it beside the
  running app as `.Minimail-update.app`. Failure opens the release page instead.
- **Restart** (`update::install`): a detached `sh` waits for the app to exit, moves the
  old bundle aside, moves the staged one in (restoring the old one if that fails) and
  relaunches it.

Ad-hoc signed builds can't verify an update and refuse to install one.
