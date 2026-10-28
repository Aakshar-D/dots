# The Releases page (updates part 2): smoke test

- Date: 2026-10-27
- Build: debug (`bun run tauri build --debug --no-bundle`), branch `feat/updates-ui` at `985977b` plus the landmark fix. Windows 11 on ARM.
- Data: a fresh sandbox folder under the session scratchpad, started with `DOTS_DATA_DIR`. No real token was used; nothing was sent to the repository except one access check with a made-up token, which GitHub refused.
- Steps: 3, as expected; one finding, fixed.

## 1. No token

Result: pass. `publisher_status` is `none`; the rail has no **Releases** icon; Settings › Updates shows **Publish versions of Dots (for its developers)…**, which opens the GitHub token chooser with "A classic token with the repo and workflow scopes.".

## 2. A made-up token

A vault entry `GITHUB_PUBLISH` with a made-up value, chosen as the publishing token and saved.

Result: pass. The check is refused with "GitHub answered 401: Bad credentials", shown under the chooser; no **Releases** icon; `/releases` says "Dots is not set up to publish. Choose a GitHub token in Settings › Updates."; the token's value is not in the day's log.

## 3. Accessibility and width

Result: Settings › Updates with the chooser open: no axe violations, dark and light. `/releases`: axe found a second `main` landmark (the page wrapped itself in `<main>` inside the app's). Fixed test-first (`ReleasesScreen.test.tsx` "sits inside the app's one main landmark"). At 420 px neither screen scrolls sideways.

The setup, trial, notes, promote, retry, drop and withdraw views are covered by `ReleasesScreen.test.tsx` against the fake backend; driving them for real needs a real token and is a person step.
