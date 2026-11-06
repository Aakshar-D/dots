# Mac build

Dots shipped on Windows only. This change makes it build and run on macOS, and makes the release
publish a Mac app beside the Windows installer. Windows behaviour is unchanged: every Mac path is
behind `cfg(target_os = "macos")` or `cfg!(target_os = "macos")`.

## What changed

**Core (`crates/dots-core`)**

- **Secrets** (`vault/secrets.rs`): `MacKeychain` keeps each vault secret in the login keychain as
  a generic password, service `Dots/<install_id>`, account `<key>` (Windows keeps them in
  Credential Manager, target `Dots/<install_id>/<key>`). Listing searches the service's items.
  Before this, secrets on macOS lasted only until Dots quit. Uses `security-framework`, a
  macOS-only dependency.
- **Org offline copy** (`vault/org/offline.rs`): `KeychainProtector` seals the copy with
  AES-256-GCM, the org id as associated data, under a random 32-byte key the keychain keeps
  (service `Dots`, account `offline-copy-key`), made on the first write. It stands in for DPAPI.
  The sealing code is tested on every platform. Before this, orgs on macOS kept no offline copy.
- **Browser use** (`browser/host.rs`): with no Edge path in Settings, macOS looks for Microsoft
  Edge, then Google Chrome (the same DevTools protocol), in `/Applications` and then
  `~/Applications`. A profile also counts as in use while its `SingletonLock` names a running
  process, which is how Edge and Chrome on macOS lock a profile.
- **Claude Code CLI** (`engine/claude/args.rs`): after PATH, macOS also looks in `~/.local/bin`,
  `~/.claude/local`, `~/.npm-global/bin`, `~/.volta/bin`, `~/.bun/bin`, `/opt/homebrew/bin` and
  `/usr/local/bin`.
- **Programs on PATH** (`mcp/stdio.rs`, `resolve_program`): outside Windows a bare name such as
  `node` or `npx` is now looked up in PATH's folders. Before, it stayed a bare name, so the
  "is it installed" check failed and browser use refused to start.
- **Folder watchers** (`watchers/folder.rs`), for what macOS's FSEvents reports: events under a
  folder's real path (`/private/var/...` for `/var/...`) are put back under the folder as the
  watcher names it; a file already reported counts as changed, not created again, and as deleted
  when it is gone (FSEvents keeps a file's "created" flag on every later event); and a rename
  that the system does not pair up counts as a creation or a deletion, depending on whether the
  path is still there.
- Already portable before this change, so left as it was: the shell tool (`sh -c` in its own
  process group outside Windows), killing a process tree, the MCP program lookup, owner-only
  file modes (`0600`), and the managed Claude Code policy folder
  (`/Library/Application Support/ClaudeCode`). `windows-sys` was already a Windows-only
  dependency.

**App (`apps/dots-app`)**

- **PATH**: an app opened from the Finder, the Dock or at login gets launchd's short PATH. At
  start, before any thread, Dots asks the login shell (`$SHELL -ilc`) for its PATH and uses it.
  A shell that fails, or takes more than 3 seconds, leaves PATH as it was.
- **Data folder**: Tauri's `local_data_dir` is `~/Library/Application Support`, so the data
  folder is `~/Library/Application Support/dots`. The early panic log now finds the same folder
  from `HOME` (on Windows it still reads `%LOCALAPPDATA%`).
- **Dock**: clicking the Dock icon brings back a window that closing hid (`RunEvent::Reopen`).
- **Autostart** already used `MacosLauncher::LaunchAgent`. The updater's quiet-install argument
  (`/S`) is Windows-only, and the updater ignores it on macOS.

**Release (`.github/workflows/release.yml`, `tools/release/release.mjs`)**

- A new `macos` job on `macos-latest` runs after the tests. It makes one universal build
  (`--target universal-apple-darwin --bundles app,dmg`), packs `Dots.app` into
  `Dots_<version>_universal.app.tar.gz`, and signs that archive with the updater key, the way
  the Windows installer is signed.
- The Windows `release` job waits for it but does not need it to succeed. When the Mac build
  succeeded, the `.dmg`, the `.app.tar.gz` and its `.sig` go into the release, and `latest.json`
  offers the archive to `darwin-aarch64` and `darwin-x86_64`. When the Mac build failed, the
  release is published without them, as before.
- `tauri.conf.json` still lists only `nsis`. The Mac job picks its own bundles with
  `--bundles`.
- `.github/workflows/mac-check.yml` runs clippy, the core tests and the app tests on macOS. It
  runs by hand (`workflow_dispatch`, once the file is on master) and on pull requests that
  change the Rust code.

## Signing

No Apple Developer ID exists for Dots, so the Mac app is **not code-signed or notarized**. The
job passes `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
`APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID` to Tauri only when those secrets exist. Adding
them turns on signing and notarization without changing the workflow. Until then:

- Gatekeeper blocks the first open. The person right-clicks Dots.app and chooses **Open**, or
  runs `xattr -d com.apple.quarantine /Applications/Dots.app`.
- The keychain ties an item's access to the app's code signature. An unsigned app that updates
  is a "different" app to the keychain, so after an update macOS may ask, once per item, whether
  Dots may read its saved secrets ("Always Allow").

The updater signature (minisign, `TAURI_SIGNING_PRIVATE_KEY`) is separate from Apple signing and
is applied to the Mac archive as it is to the Windows installer.

## Tests on macOS

The Mac check runs clippy and every core and app test on `macos-latest`, and all of them pass.
Some tests were written for Windows. They now spell paths for the system they run on: a
`C:/...` path in a test becomes `/...` elsewhere, a child's reported working folder is compared
under `/private`, and the test of Windows-only network paths runs only on Windows. One test is
skipped on macOS: `a_memory_hog_runs_out_of_memory`. There, QuickJS throws `null` when an app
uses up its memory, instead of an "out of memory" error, so a mini-app that hits the limit
reports a plain failure rather than "ran out of memory". This is still to be looked into.

## Not yet tried on a real Mac

CI compiles Dots for macOS and runs its tests there. Nobody has used the app on a Mac yet. Still
to check by hand:

- The UI in WKWebView (Safari's engine, not Edge's WebView2): layout, fonts, shortcuts (Cmd
  rather than Ctrl), the tray (menu bar) icon, and window close and reopen.
- Keychain prompts: the first save and read of a secret, and what happens after an update of
  the unsigned app.
- Browser use with real Edge and Chrome on macOS: the pipe helper, profile locking, downloads.
- The login-shell PATH with zsh, bash and fish setups, and the Claude CLI found through it.
- Autostart through the LaunchAgent, and an update installed by the updater from the
  `.app.tar.gz`.
- The `.dmg` opening and installing by drag-and-drop on Apple silicon and on Intel.
- The release workflow's `macos` job itself. It runs only for a version tag, so the universal
  build, the `.dmg` and the signed archive are first made by the next release (or a trial tag).
