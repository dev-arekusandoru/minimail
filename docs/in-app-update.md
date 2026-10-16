# In-app update (plan, not built)

Goal: nightly testers learn about new builds without checking GitHub, and later
can install them from inside the app.

## Pieces

- **`src/update.rs`**, pure logic, no GPUI. Takes `now: Timestamp` and the
  running version; the HTTP fetch sits behind a trait so tests use a fake.
- **Check**: on launch, then every 24 h, on a background thread via `ureq`:
  `GET https://api.github.com/repos/dev-arekusandoru/minimail/releases/tags/nightly`.
  Compare against the running version (`CFBundleShortVersionString`, or bake it
  in with `env!`/`option_env!`).
- **`latest.json` release asset**: the nightly workflow is a rolling release, so
  the asset URL doesn't say which build it is. Have the workflow upload
  `latest.json` (`version`, `build`, `dmg_url`, `sha256`) next to the DMG and
  have the client read that instead of parsing release titles.
- **UI**: a toast with an "Update" action when a newer build exists, plus a
  palette command "Check for updates". New actions go in `src/app/actions.rs`
  (`bind_keys` + `commands()` with a palette `Category`).

## Stages

1. **Notify only.** The action opens the release page in the browser. Small.
2. **Full auto-update.** Download the DMG to a temp dir, verify the sha256 and
   `codesign --verify` / `spctl -a`, mount it, and replace the running `.app`
   from a small detached helper script (a running app can't overwrite itself),
   then relaunch. The helper is the hard part; Sparkle isn't usable from GPUI.

Do stage 1 first; stage 2 once the pipeline has proven stable.

## Notes

- Tests use a fake fetcher, never the network.
- Respect the repo's no-system-clock rule: pass `now` in.
- Version ordering: nightlies are `<cargo version>-nightly.<YYYYMMDD>`, so
  compare the date suffix (or the build number from `latest.json`), not semver.
