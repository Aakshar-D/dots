# ChatGPT Sign-in Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan.

**Goal:** Add identity-only ChatGPT sign-in to Dots Settings.
**Architecture:** Rust owns PKCE, state, nonce, a loopback callback, token exchange, ID-token verification, and an eight-hour in-memory session. Tauri opens the system browser and returns only account display data to React.
**Tech Stack:** Rust, openidconnect, reqwest, axum, Tauri, React, TanStack Query, Vitest.
**Spec:** User requested signing into Dots with ChatGPT and confirmed that no registered client exists. Scope described in chat: Account section in Settings, browser sign-in, verified account, sign-out, and explicit unavailable state until registration.

## Constraints

- Request only openid profile email. Use Dots' own registered public client and exact callback.
- No codes, verifiers, or tokens reach the webview, logs, or persisted settings.
- Sessions expire after eight hours or app exit. Preserve local app use and model engines.
- Live sign-in requires OpenAI client registration and a consenting account.

## Review focus

Invalid signatures, issuer, audience, nonce, expiration, and subjects must fail.
Unrelated loopback requests must not consume the pending login.
Cancellation, timeout, browser failures, and concurrent attempts must clean up.
Cancelled work must never create a session.
Account changes must be separate from the Settings Save action.

## Task 1: Add account sign-in

Files: core chatgpt module and security tests; Tauri commands, setup, contract;
React Account component, Settings, schemas, wrappers, fixtures and tests;
docs/chatgpt-sign-in.md.

- [x] Write Settings tests and confirm failure because sign-in controls do not exist.
- [x] Implement native OAuth flow and signed-token security tests.
- [x] Wire commands and UI, regenerate the Rust/TypeScript contract.
- [x] Document registration, configuration, callback, and session lifetime.
- [x] Run UI tests/build and Rust checks; obtain a fresh code review.

Ruling: Prepare the configurable integration despite missing registration, because the user
authorized the feature and implementation can be reviewed and tested against a local provider.

Review fixes: publish pending cancellation atomically; prevent attempts from bypassing sign-out;
expose expires_at and rearm UI invalidation after an early expiry response. Native race
regression failed before the fix; account UI suite now passes seven behavioral tests.

Final verification:
- Full UI suite: 455 passed across 32 files.
- Full Rust workspace: 848 reported passed, 0 failed; command exited 0.
- Existing Windows symlink test reported a privilege-based skip inside its passing test.
- Production TypeScript/Vite build passed; existing bundle-size advisory remains.
- Rust/TypeScript contract regenerated and verified; Rust formatting and whitespace checked.
- Fresh review found no remaining actionable lifecycle issues.
- Live OpenAI sign-in remains untested until Dots has a registered public OAuth client.
