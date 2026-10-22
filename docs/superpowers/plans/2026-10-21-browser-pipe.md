# A Private Connection to Edge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A dot's Microsoft Edge opens no debugging port. Dots starts it through a small Node helper on Edge's debugging pipe and holds the only connection; Dots reads its pages and closes it over that pipe; the run's Playwright server reaches Edge only through a door on Dots' own server, `ws://127.0.0.1:<dots port>/cdp/<secret>`, whose fresh 128-bit secret lets exactly one connection in; and a dot's browser serves one run at a time.
**Architecture:** Two new modules in `crates/dots-core/src/browser/`: `pipe.rs` (`Frames`: NUL-ended messages put together from any pieces; `Pipe`: one Edge's connection, where Dots' own requests and the relayed requests of one Playwright get ids of the pipe's own and each answer goes back under its sender's id, events go to Playwright, and what Edge attached for a Playwright is detached when it goes) and `door.rs` (`Doors`: single-use secrets; `connect`: the axum WebSocket handler that relays a Playwright connection to its pipe). `host.rs` starts Edge through `browser-pipe.mjs` (written by Dots into `<data_dir>/browser/`, run by `node`) or, in tests, starts `fake-browser` directly on its stdin and stdout (`Launcher::Direct`); `BrowserHost::open` returns the dot's `Arc<Pipe>`. `mod.rs` opens a door per Playwright server start, names it to the server in `PLAYWRIGHT_MCP_CDP_ENDPOINT`, reads pages with `Target.getTargets`, starts a new server when a door is spent, and lets one run of a dot hold its browser. Dots' server gains the route `GET /cdp/{secret}` (`server/browser.rs`).
**Tech Stack:** Rust 1.97, tokio, axum 0.8 with its `ws` feature (tokio-tungstenite 0.29, already in axum's tree), serde_json, async-trait, thiserror; Node.js 24 for the helper (`node:child_process` only, no package). Tests: `tokio-tungstenite` 0.29 and `futures-util` 0.3 as dev-dependencies; `fake-browser` and `fake-mcp`. No UI change.
**Spec:** docs/superpowers/specs/2026-10-21-browser-design.md (section 14, "A private connection to Edge"; sections 2.1, 5, 9, 11 and 13 as it changes them)

## Global Constraints

- Repository root `C:/Users/shrad/OneDrive/Documents/LOQ/Dots/dots`, branch `feat/browser`; all commands run from the root unless a step says otherwise. Never commit to `master`. `git add` names its files: never `git add -A` or `git add .`. No `Co-Authored-By` line or any other AI trailer in any commit message.
- Toolchain: Rust 1.97 stable; Node.js on `PATH` as `node` (24.18 on this PC); Bun 1.4.3 at `~/.bun/bin` only if the UI is touched (it is not); Windows 11 on ARM, Git Bash. Every command that builds `dots-app` (`cargo test -p dots-app`, `cargo clippy --workspace --all-targets`) runs with `export PATH="/c/Program Files/LLVM/bin:$PATH"` first (the `ring` crate needs clang). `cargo test -p dots-app` and clippy need `ui/dist`: in a fresh checkout run `bun install --frozen-lockfile`, then `bun x vite build` in `ui`, first.
- Tests need Node.js from Task 3 on: `browser_host_test::the_helper_gives_edge_its_pipe_and_closes_it_when_dots_goes` and the three `runtime_browser_test` tests start `fake-browser` through the real helper, as Dots starts Edge. Every other browser test starts `fake-browser` directly (`common::fake_browser::host`, `Launcher::Direct`). Tests never start the real Edge, the real `npx` or the network.
- A parallel developer changes approvals, the scheduler and the gateway. Shared files get the smallest edits: `runtime.rs` (one argument), `server/mod.rs` (one route and its comment), `server/browser.rs` (imports, one handler appended), `src/bin/fake-mcp.rs` (one filter and its doc line). `runner.rs`, `server/gateway.rs`, `server/mcp.rs`, `run_tools.rs`, `approvals.rs` and `engine/` are not changed. Logic goes in `crates/dots-core/src/browser/{pipe.rs,door.rs,host.rs,mod.rs,browser-pipe.mjs}` and `src/bin/fake-browser.rs`.
- The pipe (spec section 14): messages are JSON, each ended by one NUL byte, both ways. `browser::pipe::MAX_MESSAGE` = 256 MiB: a longer message from Edge is dropped up to its NUL; the WebSocket takes messages and frames up to that size. Dots' own requests: `Browser.getVersion` (at a start, waiting `host::START_WAIT`, 20 s; and before a running Edge is used again, waiting `ASK_WAIT`, so that a window the person just closed is started again), `Target.getTargets` (the pages: `targetInfos` of type `page`), `Browser.close` (waiting `pipe::ASK_WAIT`, 5 s); when a Playwright goes, `Target.detachFromTarget` for each session Edge attached for it at the top level, then `Target.setAutoAttach {"autoAttach": false, "waitForDebuggerOnStart": false}`, unanswered.
- The helper `browser-pipe.mjs` (`host::HELPER`, written to `<data_dir>/browser/browser-pipe.mjs` before each start; `host::HELPER_FILE`) runs as `node browser-pipe.mjs <msedge.exe> <switches>` and starts Edge with `host::PIPE_FLAGS` = `--remote-debugging-pipe --edge-skip-compat-layer-relaunch` before Dots' switches, on its descriptors 3 and 4. When its stdin ends it ends a message cut short with a NUL, sends `{"id":2147483647,"method":"Browser.close"}`, and kills Edge after 5 s; it exits when Edge exits.
- Edge's switches: `--user-data-dir=<data_dir>/browser/<dot_id>/profile`, `host::EDGE_FLAGS` (unchanged), `about:blank`. No `--remote-debugging-port`, no `--remote-debugging-address`, no `port` file.
- The door (spec section 14): a secret is 16 bytes from `getrandom`, base64url without padding (22 characters; `door::new_secret`). Its route is `GET /cdp/{secret}` on Dots' server, behind `loopback_only`. A wrong or spent secret, or a second connection, answers 404 and logs `refused a connection to a dot's browser: its secret is wrong or spent` (never the secret). A request with the right secret that is not a WebSocket upgrade spends the door and gets the upgrade's own rejection. The Playwright server is told its door in the variable `browser::CDP_ENDPOINT_VAR` = `PLAYWRIGHT_MCP_CDP_ENDPOINT` (read by `@playwright/mcp@0.0.83`), not on its command line; its arguments become `-y @playwright/mcp@0.0.83 --output-dir <run workspace>/downloads --snapshot-mode none`.
- Exact sentences: a second run of a dot `This dot's browser is in use by another run; try again when it ends.` (`HostError::InUse`); Node.js missing `Browser use needs Node.js (npx).` (`HostError::NoNode`, the same text as `browser::NO_NODE`). The other sentences of the browser plan are unchanged.
- How the code blocks are written. A step changes files in three ways, applied in the order given:
  - **Create `path`:** the block is the whole new file (for `host.rs`, `fake-browser.rs` and `browser_host_test.rs`, it replaces the file).
  - **In `path`, replace:** … **with:** … The first block occurs in the file exactly once, as it stands at that moment; the second takes its place.
  - **Append to `path`:** the block is added at the end of the file.
- A Replace whose passage is not found once (the parallel developer changed it): stop, read the file, and apply the same change to the passage as it is now; never paste over their change.
- Before each commit: the task's own tests, then `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app` (from Task 3, which changes `runtime.rs` and the server), `cargo clippy --workspace --all-targets` (no warnings), `cargo fmt` then `cargo fmt --check`. Files are LF with a final newline and valid UTF-8 (`git diff --cached --check`). No `fake-browser` process is left after a test run: `tasklist | grep -c fake-browser` prints `0`.
- Known failures, not caused here: `runtime_review_test::preview_schedule_lists_the_next_fire_times` fails at `5de4432` in a +05:30 time zone; under load `approvals_test::ask_approved_within_window_is_live`, `runtime_local_test::live_approval_lets_the_command_run`, `local_tools_test::shell_stops_at_its_timeout_and_on_cancel`, `claude_engine_test::a_finished_run_leaves_no_process_behind` and some `resume_test` tests can fail. Rerun alone before treating one as a failure.

## Review Focus

The inputs and failure modes the spec implies that no task's tests would naturally exercise, most likely to bite a person first, each pinned to the test that covers it:

1. **Who comes through a door.** Two clients racing for one secret; Playwright reconnecting with its secret after its connection dropped; a wrong secret; a plain HTTP request with the right secret; the route on the real server without browsers' secret. Expected: exactly one of the racing clients gets in and the other gets 404; a spent or wrong secret is 404; the plain request spends the door; the run's next call starts a new Playwright server with a new secret, and the old one stays out. Tests: `browser_door_test::a_secret_lets_one_connection_in_once` (Task 2), `claude_browser_test::the_server_refuses_a_door_without_its_secret` (Task 3), `browser_door_test::a_run_whose_playwright_lost_its_door_gets_a_new_one` (Task 4).
2. **Ids colliding.** Playwright numbering its requests from 1 as the pipe does; Dots asking while Playwright's requests are in flight; answers out of order; an answer under an id nobody asked with; an error answer to Dots; a session id on a request; a second Playwright arriving before the first one's end was seen. Expected: every request reaches Edge under a distinct id; each answer goes back to its sender under the sender's id; the stray answer goes nowhere; the first Playwright's later requests are dropped. Tests: `browser_pipe_test::dots_and_playwright_get_their_own_answers_whatever_the_ids` and `browser_pipe_test::what_edge_attached_for_a_playwright_is_undone_when_it_goes` (Task 1).
3. **Edge closed mid-relay.** Half a message, then Edge's end of the pipe closes, while Dots' `Target.getTargets` and Playwright's request wait; the person closing the window while Playwright is connected; a door opened to an Edge that has gone. Expected: Dots' request fails at once with `Unreachable`, Playwright's WebSocket closes, the door is spent, the pipe takes no new client, and the next `open` starts Edge again. Tests: `browser_pipe_test::edge_closing_mid_relay_ends_every_wait` (Task 1), `browser_door_test::edge_closing_mid_relay_ends_the_connection` (Task 2), `browser_host_test::closing_ends_it_over_its_pipe_and_a_closed_window_starts_again` (Task 3).
4. **Framing and size.** A message cut anywhere, several in one read, empty messages, a message longer than the limit (dropped up to its NUL, the next one read), a raw NUL inside Playwright's text, a `\u0000` escape (kept escaped on the pipe), Playwright text that is not JSON, not an object, or has no number `id`; a 20 MB message from Playwright and a 30 MB one from Edge through the WebSocket (tungstenite's defaults stop at 16 MiB frames). Expected: whole messages only, garbage dropped and logged, the connection still working. Tests: `browser_pipe_test::messages_are_put_together_from_any_pieces`, `browser_pipe_test::garbage_is_dropped_and_huge_messages_arrive_whole` (Task 1), `browser_door_test::playwright_and_edge_talk_through_the_door` (Task 2).
5. **Lifetimes.** A Playwright that goes and leaves its sessions attached (measured: the next `connectOverCDP` then saw no page); two runs of one dot at once; a refused run ending; the person's "Open the browser" during a run; Dots stopping with Edge open. Expected: the sessions are detached and auto-attach is off before the next Playwright's first request; the second run's call fails with the in-use sentence and starts nothing; the refused run's end leaves the holder's Edge open; the person's Edge stays after the run; Edge closes when Dots stops (the helper's stdin ends). Tests: `browser_door_test::playwright_and_edge_talk_through_the_door` (Task 2), `browser_host_test::the_helper_gives_edge_its_pipe_and_closes_it_when_dots_goes` and `browser_host_test::a_second_open_finds_it_and_a_dots_that_starts_again_starts_another` (Task 3), `browser_call_test::a_dots_browser_serves_one_run_at_a_time` (Task 4).

---

## Context

It builds section 14 of the browser spec, agreed after the browser plan's final review (finding 7: Edge's debugging port had no password). The browser plan `docs/superpowers/plans/2026-10-21-browser.md` built sections 1 to 13; this plan follows its form. Task 5 brings the spec in line with what the tasks build; the table "Deviations" lists what changed and why.

**What was measured before this plan was written** (2026-10-21, Edge 155.0.4283.45 on Windows 11 ARM, Node.js 24.18, `@playwright/mcp@0.0.83` with its Playwright 1.64 from the npx cache, throwaway profiles under the session's scratch folder, never the person's):

- The spike: Node's `spawn(msedge, ['--remote-debugging-pipe', '--user-data-dir=…', '--no-first-run', '--no-default-browser-check', '--disable-sync', '--edge-skip-compat-layer-relaunch', 'about:blank'], {stdio: ['ignore', 'ignore', 'ignore', 'pipe', 'pipe']})` answered `Browser.getVersion`, `Target.getTargets` and `Browser.close` over descriptors 3 and 4; without `--edge-skip-compat-layer-relaunch` Edge exited at once.
- A relay prototype (the helper above, a WebSocket server, ids renumbered) under Playwright's own `chromium.connectOverCDP`, three times in a row on one Edge: each connection saw the page the one before left, navigated, and read an aria snapshot. Without undoing what the earlier Playwright set up (detaching the sessions Edge attached for it, auto-attach off), the second connection saw no page and the third none. A relay that gave each Playwright a session of its own (`Target.attachToBrowserTarget`) timed out in `connectOverCDP`; it is not used.
- A Rust program started by Node with pipes on descriptors 3 and 4 reads and writes them through the C runtime's `_get_osfhandle(3)` and `_get_osfhandle(4)`: that is how `fake-browser` serves the helper in tests.
- A draft of this plan's code (it named the door on Playwright's command line, and `open` did not yet ask a running Edge whether it still answers), applied to a copy of the branch: `EdgeHost::new` (the helper under `node`, its console hidden) started the real Edge; its pages, `Browser.getVersion` and a relayed `Target.setAutoAttach` answered over the pipe; no `msedge.exe` whose command line named the profile held a listening TCP port (`Get-NetTCPConnection -State Listen -OwningProcess <pid>` for each); `close` ended every process of that profile; dropping the host (Dots stopping) also ended them. Real Playwright 1.64 through the Rust door (`door::connect` on a test server), three successive processes, each `connectOverCDP`, `goto`, aria snapshot and a full-page screenshot, each process ending abruptly as Dots ends a server: every one saw the page the one before left.
- `@playwright/mcp@0.0.83` reads `PLAYWRIGHT_MCP_CDP_ENDPOINT` as `--cdp-endpoint` (`options.cdpEndpoint = envToString(e.PLAYWRIGHT_MCP_CDP_ENDPOINT)` in its `coreBundle.js`). A process's command line is listed to any program by `Get-CimInstance Win32_Process`.
- tungstenite 0.29 takes frames of 16 MiB and messages of 64 MiB by default; Playwright's own WebSocket client takes up to 256 MiB.

**How this plan's code was written:** every Replace block below was copied from the files at commit `5de4432` (branch `feat/browser`), or from the files as an earlier task of this plan leaves them. Before the plan was handed over, all of its blocks were applied in order to a copy of `5de4432` outside the repository: every Replace passage was found exactly once, and no task's blocks needed a `cargo fmt` change. Each task's Step 2 failed as written and its Step 4 passed; after each task `cargo clippy --workspace --all-targets` gave no warning and no `fake-browser` process was left. After Task 4, `cargo test -p dots-core --no-fail-fast` passed 792 tests with the one known failure (`runtime_review_test::preview_schedule_lists_the_next_fire_times`), `cargo test -p dots-app` passed (95 tests) and `cargo fmt --check` passed; Task 5's checks passed. The UI is not touched. A step that still fails is fixed in that step, keeping the step's intent, and the fix is named in the task's commit message body.

## Deviations from the spec

The spec says something else, or nothing, in these places; Task 5 records them in section 14.

| Spec | This plan | Why |
|---|---|---|
| §14: the Playwright server "is started with `--cdp-endpoint ws://127.0.0.1:<dots port>/cdp/<secret>`" | The same address in the variable `PLAYWRIGHT_MCP_CDP_ENDPOINT`, which 0.0.83 reads as `--cdp-endpoint` | A command line is listed to any program on the computer (`Get-CimInstance Win32_Process`); the secret would be there for a dot's shell tool to read and race Playwright with |
| §14: "Dots relays …; events go to Playwright" | Also: when a Playwright goes, Dots detaches the sessions Edge attached for it at the top level and turns auto-attach off; a new Playwright on the same pipe lets the one before go first | Measured: otherwise the next Playwright's `connectOverCDP` found no page. The pipe is one DevTools client for the whole life of Edge; a WebSocket's end would have undone this by itself |
| §14: "a second connection, a wrong or a spent secret is refused (HTTP 404)" | Also a request with the right secret that is not a WebSocket upgrade spends the door; it gets the upgrade's own rejection (a 4xx), not 404 | The door is single-use whatever comes through it first |
| §14: silent on a Playwright whose connection dropped | The run's next browser call stops that server and starts a new one with a new door | Its secret is spent: it could never come in again |
| §14: "Each run's browser gets a fresh random secret … when its Playwright server starts" | Each start of a Playwright server gets one (a run whose server stopped or lost its door gets another) | The same rule, applied to every start |
| §14: "Edge lives as long as Dots"; §2.1: "Edge is closed when the last run of the dot that used it ends" | Edge closes when the run that holds it ends (unless the person opened it), and in any case when Dots stops | One run holds a dot's browser at a time (§14) |
| §14: silent on sizes | A message of up to 256 MB either way; a longer one from Edge is dropped and logged | A full-page screenshot is several MB; tungstenite's defaults stop at 16 MiB |
| §14 Errors: "Node not found: the call fails with `NO_NODE`" | `HostError::NoNode`, whose text is `NO_NODE`'s | Node starts the helper, before Playwright's `npx` |
| §14 Testing: "a fake Edge that speaks the pipe protocol (`fake-browser`, reworked)" | `fake-browser` speaks the pipe on stdin and stdout when the test starts it, and on descriptors 3 and 4 when the real helper starts it; its test controls (`/fake/pages`, `/fake/quit`) are on an HTTP port of its own that Dots never uses; the pipe's own logic is tested with the test playing Edge in memory (`common::edge_pipe::TestEdge`) | The helper is tested as it runs; the controls are the test's, not Edge's |
| §14: silent on Dots' server being down | A Playwright server cannot reach its door, so a local dot's browser call fails too (with Playwright's connection error) | The door is on Dots' server; before, a local dot's browser did not need it |

## File Map

- `crates/dots-core/src/browser/pipe.rs` (new, Task 1): `Frames`, `Pipe`.
- `crates/dots-core/src/browser/door.rs` (new, Task 2): `new_secret`, `Doors`, `connect`.
- `crates/dots-core/src/browser/mod.rs` (Tasks 1, 2: the module list; Task 3: `CDP_ENDPOINT_VAR`, `args_for`, `Browsers::{new, doors}`, `RunBrowser` on the pipe and its doors; Task 4: one run at a time, a spent door).
- `crates/dots-core/src/browser/host.rs` (rewritten, Task 3; `InUse` in Task 4), `crates/dots-core/src/browser/browser-pipe.mjs` (new, Task 3).
- `crates/dots-core/src/bin/fake-browser.rs` (rewritten, Task 3), `crates/dots-core/src/bin/fake-mcp.rs` (Task 3: it records `PLAYWRIGHT_MCP_*` variables).
- `crates/dots-core/src/server/mod.rs`, `crates/dots-core/src/server/browser.rs`, `crates/dots-core/src/runtime.rs` (Task 3).
- `crates/dots-core/Cargo.toml`, `Cargo.lock` (Task 2).
- Tests: `crates/dots-core/tests/browser_pipe_test.rs` (new, Task 1), `tests/common/edge_pipe.rs` (new, Task 1), `tests/common/mod.rs` (Task 1), `tests/browser_door_test.rs` (new, Task 2; grown in Task 4), `tests/browser_host_test.rs` (rewritten, Task 3), `tests/common/fake_browser.rs`, `tests/browser_call_test.rs`, `tests/claude_browser_test.rs`, `tests/local_browser_test.rs`, `tests/runtime_browser_test.rs` (Task 3; `browser_call_test.rs` again in Task 4).
- Docs: `docs/superpowers/specs/2026-10-21-browser-design.md` (Task 5).

### Task 1: The pipe: messages, ids, and what a Playwright set up

`pipe.rs` is one Edge's connection, over any reader and writer (the helper's stdout and stdin in Task 3; an in-memory pipe in this task's tests). `Frames` puts NUL-ended messages together from whatever pieces a pipe delivers. `Pipe` gives every request an id of its own: Dots' requests (`ask`, `pages`) wait for their answers; the requests of the one connected Playwright (`connect`, `relay`) are answered back under the ids Playwright gave; events go to Playwright; when Playwright goes (`disconnect`, or a new one connecting), the sessions Edge attached for it are detached and auto-attach is turned off. When Edge's end closes, every wait fails and Playwright's connection ends.

**Files:**

- Create: `crates/dots-core/src/browser/pipe.rs`
- Modify: `crates/dots-core/src/browser/mod.rs` (the module list)
- Create: `crates/dots-core/tests/common/edge_pipe.rs`
- Modify: `crates/dots-core/tests/common/mod.rs` (the module list)
- Test: `crates/dots-core/tests/browser_pipe_test.rs` (new)

**Interfaces:**

- Consumes: `host::{page_urls, HostError}` (browser plan Task 5); tokio's `AsyncRead`/`AsyncWrite`, `mpsc`, `oneshot`; `CancellationToken`.
- Produces (in `dots_core::browser::pipe`):
  - `pub const MAX_MESSAGE: usize` (256 MiB), `pub const ASK_WAIT: Duration` (5 s).
  - `#[derive(Debug)] pub struct Frames` with `Default` (the limit `MAX_MESSAGE`), `fn with_max(max: usize) -> Frames`, `fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>>`.
  - `pub struct Pipe` (`Debug`) with `fn new<R: AsyncRead + Unpin + Send + 'static, W: AsyncWrite + Unpin + Send + 'static>(from_edge: R, to_edge: W) -> Arc<Pipe>`, `fn is_closed(&self) -> bool`, `async fn closed(&self)`, `async fn ask(&self, method: &str, params: Value) -> Result<Value, HostError>`, `async fn ask_within(&self, method: &str, params: Value, wait: Duration) -> Result<Value, HostError>`, `async fn pages(&self) -> Result<Vec<String>, HostError>`, `fn connect(&self) -> Option<(u64, mpsc::UnboundedReceiver<String>)>`, `fn relay(&self, serial: u64, text: &str) -> bool`, `fn disconnect(&self, serial: u64)`.
  - Tests: `common::edge_pipe::TestEdge` with `start() -> TestEdge`, `pipe: Arc<Pipe>`, `next`, `next_within`, `send`, `send_raw`, `close`.

- [ ] **Step 1: Write the failing tests**

**Create `crates/dots-core/tests/common/edge_pipe.rs`:**

```rust
//! An Edge played by the test over an in-memory pipe (`browser::pipe::Pipe`): each message Dots
//! sent, in order, and whatever the test sends back, as messages or as raw bytes.

use std::sync::Arc;
use std::time::Duration;

use dots_core::browser::pipe::{Frames, Pipe};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, WriteHalf};
use tokio::sync::mpsc;

pub struct TestEdge {
    pub pipe: Arc<Pipe>,
    from_dots: mpsc::UnboundedReceiver<Value>,
    to_dots: WriteHalf<DuplexStream>,
}

impl TestEdge {
    pub fn start() -> TestEdge {
        let (dots_side, edge_side) = tokio::io::duplex(64 * 1024);
        let (from_edge, to_edge) = tokio::io::split(dots_side);
        let pipe = Pipe::new(from_edge, to_edge);
        let (mut edge_in, to_dots) = tokio::io::split(edge_side);
        let (tx, from_dots) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut frames = Frames::default();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = match edge_in.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                for frame in frames.push(&buf[..n]) {
                    let _ = tx.send(serde_json::from_slice(&frame).expect("Dots sends JSON"));
                }
            }
        });
        TestEdge {
            pipe,
            from_dots,
            to_dots,
        }
    }

    /// The next message Dots sent; fails after 5 s.
    pub async fn next(&mut self) -> Value {
        self.next_within(Duration::from_secs(5))
            .await
            .expect("a message from Dots within 5 s")
    }

    /// The next message Dots sends within `wait`, if any.
    pub async fn next_within(&mut self, wait: Duration) -> Option<Value> {
        tokio::time::timeout(wait, self.from_dots.recv())
            .await
            .ok()
            .flatten()
    }

    /// Bytes as Edge writes them: a message may come in pieces, or several at once.
    pub async fn send_raw(&mut self, bytes: &[u8]) {
        self.to_dots.write_all(bytes).await.unwrap();
        self.to_dots.flush().await.unwrap();
    }

    /// One message, ended by its NUL byte.
    pub async fn send(&mut self, message: &Value) {
        let mut bytes = message.to_string().into_bytes();
        bytes.push(0);
        self.send_raw(&bytes).await;
    }

    /// Edge ends: its end of the pipe closes.
    pub async fn close(&mut self) {
        let _ = self.to_dots.shutdown().await;
    }
}
```

**In `crates/dots-core/tests/common/mod.rs`, replace:**

```rust
pub mod clock;
pub mod fake_browser;
```

**with:**

```rust
pub mod clock;
pub mod edge_pipe;
pub mod fake_browser;
```

**Create `crates/dots-core/tests/browser_pipe_test.rs`:**

```rust
//! A dot's Edge over its debugging pipe (browser spec section 14), with the test playing Edge:
//! messages put together from the pieces a pipe delivers; Dots' own requests and Playwright's
//! under ids that never collide; events to Playwright; what Edge attached for a Playwright
//! undone when it goes; garbage, huge messages, and Edge closing mid-relay.

mod common;

use std::collections::HashSet;
use std::time::Duration;

use common::edge_pipe::TestEdge;
use dots_core::browser::host::HostError;
use dots_core::browser::pipe::{Frames, MAX_MESSAGE};
use serde_json::{json, Value};

fn text(frames: Vec<Vec<u8>>) -> Vec<String> {
    frames
        .into_iter()
        .map(|f| String::from_utf8(f).unwrap())
        .collect()
}

#[test]
fn messages_are_put_together_from_any_pieces() {
    assert_eq!(MAX_MESSAGE, 256 * 1024 * 1024);
    let mut frames = Frames::default();
    // Cut anywhere, several at once, and empty ones between.
    assert!(frames.push(br#"{"id":1,"res"#).is_empty());
    assert_eq!(
        text(frames.push(b"ult\":{}}\0\0{\"id\":2}\0{\"id\"")),
        [r#"{"id":1,"result":{}}"#, r#"{"id":2}"#]
    );
    assert_eq!(text(frames.push(b":3}\0")), [r#"{"id":3}"#]);
    assert!(frames.push(b"\0\0").is_empty());
    // One longer than the most allowed is dropped up to its NUL, in whatever pieces; the next
    // one is read.
    let mut small = Frames::with_max(8);
    assert!(small.push(b"12345").is_empty());
    assert!(small.push(b"6789").is_empty());
    assert!(small.push(b"0123").is_empty());
    assert_eq!(text(small.push(b"45\0short\0")), ["short"]);
    assert_eq!(text(small.push(b"12345678\0")), ["12345678"]);
    assert_eq!(text(small.push(b"123456789\0ok\0")), ["ok"]);
}

#[tokio::test]
async fn dots_and_playwright_get_their_own_answers_whatever_the_ids() {
    let mut edge = TestEdge::start();
    let (serial, mut playwright) = edge.pipe.connect().unwrap();
    // Playwright numbers its requests from 1, as Dots' pipe does.
    assert!(edge.pipe.relay(
        serial,
        r#"{"id":1,"method":"Page.navigate","sessionId":"S1","params":{"url":"https://shop.com/"}}"#
    ));
    let pipe = edge.pipe.clone();
    let pages = tokio::spawn(async move { pipe.pages().await });
    assert!(edge
        .pipe
        .relay(serial, r#"{"id":2,"method":"Browser.getVersion"}"#));
    let mut sent = vec![edge.next().await, edge.next().await, edge.next().await];
    let ids: HashSet<u64> = sent.iter().map(|m| m["id"].as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 3, "{sent:?}");
    sent.sort_by_key(|m| m["method"].as_str().unwrap().to_string());
    let [version, navigate, targets] = [sent[0].clone(), sent[1].clone(), sent[2].clone()];
    assert_eq!(version["method"], "Browser.getVersion");
    assert_eq!(navigate["sessionId"], "S1");
    assert_eq!(navigate["params"]["url"], "https://shop.com/");
    assert_eq!(targets["method"], "Target.getTargets");
    // Answered out of order; an answer under an id nobody asked with goes nowhere.
    edge.send(&json!({"id": 999_999, "result": {}})).await;
    edge.send(&json!({"id": version["id"], "result": {"product": "Edg/1"}}))
        .await;
    edge.send(&json!({"id": targets["id"], "result": {"targetInfos": [
        {"targetId": "P1", "type": "page", "url": "https://shop.com/cart"},
        {"targetId": "SW", "type": "service_worker", "url": "chrome-extension://x/bg.js"}
    ]}}))
    .await;
    edge.send(&json!({"id": navigate["id"], "sessionId": "S1", "result": {"frameId": "F1"}}))
        .await;
    assert_eq!(
        pages.await.unwrap().unwrap(),
        ["https://shop.com/cart".to_string()]
    );
    let first: Value = serde_json::from_str(&playwright.recv().await.unwrap()).unwrap();
    let second: Value = serde_json::from_str(&playwright.recv().await.unwrap()).unwrap();
    assert_eq!(first, json!({"id": 2, "result": {"product": "Edg/1"}}));
    assert_eq!(
        second,
        json!({"id": 1, "sessionId": "S1", "result": {"frameId": "F1"}})
    );
    // Events go to Playwright as they came; an error answer to Dots is an error.
    edge.send(&json!({"method": "Page.loadEventFired", "sessionId": "S1", "params": {}}))
        .await;
    assert_eq!(
        serde_json::from_str::<Value>(&playwright.recv().await.unwrap()).unwrap()["method"],
        "Page.loadEventFired"
    );
    let pipe = edge.pipe.clone();
    let failing = tokio::spawn(async move { pipe.ask("Browser.close", json!({})).await });
    let close = edge.next().await;
    edge.send(&json!({"id": close["id"], "error": {"code": -32000, "message": "no"}}))
        .await;
    assert!(matches!(
        failing.await.unwrap(),
        Err(HostError::Unreachable(_))
    ));
}

#[tokio::test]
async fn what_edge_attached_for_a_playwright_is_undone_when_it_goes() {
    let mut edge = TestEdge::start();
    let (first, mut old) = edge.pipe.connect().unwrap();
    for (session, method) in [
        ("S1", "Target.attachedToTarget"),
        ("S2", "Target.attachedToTarget"),
        ("S3", "Target.attachedToTarget"),
        ("S2", "Target.detachedFromTarget"),
    ] {
        edge.send(&json!({"method": method, "params": {"sessionId": session}}))
            .await;
    }
    // A worker attached inside a page's session is that session's, not a top-level one.
    edge.send(
        &json!({"method": "Target.attachedToTarget", "sessionId": "S1",
                      "params": {"sessionId": "W1"}}),
    )
    .await;
    for _ in 0..5 {
        old.recv().await.unwrap();
    }
    // A new Playwright comes before the old one's end was seen: the old one is let go, and
    // its sessions are detached before the new one's first request.
    let (second, mut new) = edge.pipe.connect().unwrap();
    assert!(old.recv().await.is_none(), "the old connection ends");
    assert!(!edge
        .pipe
        .relay(first, r#"{"id":5,"method":"Browser.getVersion"}"#));
    assert!(edge
        .pipe
        .relay(second, r#"{"id":1,"method":"Target.setAutoAttach"}"#));
    let undone: Vec<Value> = vec![edge.next().await, edge.next().await, edge.next().await];
    assert_eq!(undone[0]["method"], "Target.detachFromTarget");
    assert_eq!(undone[0]["params"]["sessionId"], "S1");
    assert_eq!(undone[1]["params"]["sessionId"], "S3");
    assert_eq!(undone[2]["method"], "Target.setAutoAttach");
    assert_eq!(
        undone[2]["params"],
        json!({"autoAttach": false, "waitForDebuggerOnStart": false})
    );
    // Their answers go nowhere.
    for message in &undone {
        edge.send(&json!({"id": message["id"], "result": {}})).await;
    }
    let request = edge.next().await;
    assert_eq!(request["method"], "Target.setAutoAttach");
    edge.send(&json!({"id": request["id"], "result": {}})).await;
    assert_eq!(new.recv().await.unwrap(), r#"{"id":1,"result":{}}"#);
    // The one that went is undone once; a stale serial changes nothing.
    edge.pipe.disconnect(first);
    edge.pipe.disconnect(second);
    let last = edge.next().await;
    assert_eq!(last["params"]["autoAttach"], false);
    assert!(edge.next_within(Duration::from_millis(200)).await.is_none());
}

#[tokio::test]
async fn garbage_is_dropped_and_huge_messages_arrive_whole() {
    let mut edge = TestEdge::start();
    let (serial, mut playwright) = edge.pipe.connect().unwrap();
    // From Playwright: not JSON (a raw NUL in it), no id, not an object.
    assert!(!edge
        .pipe
        .relay(serial, "{\"id\":7,\"method\":\"Fake\0.x\"}"));
    assert!(!edge
        .pipe
        .relay(serial, r#"{"method":"Browser.getVersion"}"#));
    assert!(!edge.pipe.relay(serial, "[1,2]"));
    assert!(!edge.pipe.relay(serial, r#"{"id":"x","method":"A.b"}"#));
    assert!(edge.next_within(Duration::from_millis(200)).await.is_none());
    // A NUL inside a JSON string is escaped on the way: the pipe's framing holds.
    assert!(edge.pipe.relay(
        serial,
        r#"{"id":8,"method":"Runtime.evaluate","params":{"expression":"'\u0000'"}}"#
    ));
    let evaluate = edge.next().await;
    assert_eq!(evaluate["params"]["expression"], "'\u{0}'");
    // From Edge: not JSON, not an object; then a 20 MB answer in three pieces.
    edge.send_raw(b"not json\0[1]\0").await;
    let big = format!(
        "{{\"id\":{},\"result\":{{\"data\":\"{}\"}}}}\0",
        evaluate["id"],
        "x".repeat(20_000_000)
    );
    let (a, rest) = big.as_bytes().split_at(7);
    let (b, c) = rest.split_at(10_000_000);
    edge.send_raw(a).await;
    edge.send_raw(b).await;
    edge.send_raw(c).await;
    let answer: Value = serde_json::from_str(&playwright.recv().await.unwrap()).unwrap();
    assert_eq!(answer["id"], 8);
    assert_eq!(answer["result"]["data"].as_str().unwrap().len(), 20_000_000);
}

#[tokio::test]
async fn edge_closing_mid_relay_ends_every_wait() {
    let mut edge = TestEdge::start();
    let (serial, mut playwright) = edge.pipe.connect().unwrap();
    assert!(edge
        .pipe
        .relay(serial, r#"{"id":1,"method":"Page.navigate"}"#));
    let pipe = edge.pipe.clone();
    let pages = tokio::spawn(async move { pipe.pages().await });
    edge.next().await;
    edge.next().await;
    // Half a message, then Edge is gone.
    edge.send_raw(br#"{"id":1,"res"#).await;
    edge.close().await;
    assert!(matches!(
        pages.await.unwrap(),
        Err(HostError::Unreachable(_))
    ));
    assert!(playwright.recv().await.is_none());
    edge.pipe.closed().await;
    assert!(edge.pipe.is_closed());
    assert!(edge.pipe.connect().is_none());
    assert!(!edge
        .pipe
        .relay(serial, r#"{"id":2,"method":"Browser.getVersion"}"#));
    assert!(matches!(
        edge.pipe.ask("Browser.getVersion", json!({})).await,
        Err(HostError::Unreachable(_))
    ));
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test browser_pipe_test`
Expected: FAIL to compile: unresolved import `dots_core::browser::pipe` (could not find `pipe` in `browser`).

- [ ] **Step 3: Add the pipe**

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
pub mod host;
pub mod sites;
```

**with:**

```rust
pub mod host;
pub mod pipe;
pub mod sites;
```

**Create `crates/dots-core/src/browser/pipe.rs`:**

```rust
//! A dot's Edge over its debugging pipe (browser spec section 14). Edge is started with
//! `--remote-debugging-pipe` and opens no port: Dots holds its only connection. Messages are
//! JSON, each ended by a NUL byte. Dots' own requests (the pages, closing) and the requests of
//! the run's Playwright, relayed from its door (`door.rs`), share the pipe: every request gets
//! an id of the pipe's own, and each answer goes back to its sender under the id the sender
//! gave. Events go to Playwright.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::host::{page_urls, HostError};

/// The longest message either side may send: a full-page screenshot is a few MB.
pub const MAX_MESSAGE: usize = 256 * 1024 * 1024;
/// How long Dots waits for Edge to answer one of its own requests.
pub const ASK_WAIT: Duration = Duration::from_secs(5);

/// NUL-ended messages, put together from the pieces a pipe delivers.
#[derive(Debug)]
pub struct Frames {
    buf: Vec<u8>,
    max: usize,
    /// The message being read is longer than `max`: it is dropped up to its NUL.
    skipping: bool,
}

impl Default for Frames {
    fn default() -> Self {
        Frames::with_max(MAX_MESSAGE)
    }
}

impl Frames {
    /// Frames whose messages may be `max` bytes long at most.
    pub fn with_max(max: usize) -> Frames {
        Frames {
            buf: Vec::new(),
            max,
            skipping: false,
        }
    }

    /// Takes the next bytes read; returns every message they complete, in order. An empty
    /// message is skipped, and one longer than the most allowed is dropped.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut done = Vec::new();
        for part in bytes.split_inclusive(|b| *b == 0) {
            let ended = part.last() == Some(&0);
            let body = if ended { &part[..part.len() - 1] } else { part };
            if !self.skipping {
                if self.buf.len() + body.len() > self.max {
                    tracing::warn!("dropped a browser message longer than {} bytes", self.max);
                    self.buf = Vec::new();
                    self.skipping = true;
                } else {
                    self.buf.extend_from_slice(body);
                }
            }
            if ended {
                if !self.skipping && !self.buf.is_empty() {
                    done.push(std::mem::take(&mut self.buf));
                }
                self.skipping = false;
            }
        }
        done
    }
}

/// The run's Playwright, connected through a door.
struct Client {
    serial: u64,
    to: mpsc::UnboundedSender<String>,
    /// The pipe's id of each of its requests in flight, and the id it gave.
    ids: HashMap<u64, Value>,
    /// The sessions Edge attached for it at the top level (`Target.attachedToTarget`).
    sessions: BTreeSet<String>,
}

#[derive(Default)]
struct Waiting {
    /// The pipe ended: nothing more is answered.
    ended: bool,
    own: HashMap<u64, oneshot::Sender<Value>>,
    client: Option<Client>,
}

/// The connection to one Edge.
pub struct Pipe {
    to_edge: mpsc::UnboundedSender<Vec<u8>>,
    next: AtomicU64,
    waiting: Mutex<Waiting>,
    gone: CancellationToken,
}

impl std::fmt::Debug for Pipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipe")
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl Pipe {
    /// The pipe read from `from_edge` and written to `to_edge` (the helper's stdout and
    /// stdin). It ends when either does; Edge's input closes once the last `Arc` of the pipe
    /// is dropped.
    pub fn new<R, W>(from_edge: R, to_edge: W) -> Arc<Pipe>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (tx, rx) = mpsc::unbounded_channel();
        let pipe = Arc::new(Pipe {
            to_edge: tx,
            next: AtomicU64::new(1),
            waiting: Mutex::new(Waiting::default()),
            gone: CancellationToken::new(),
        });
        tokio::spawn(write(rx, to_edge, Arc::downgrade(&pipe)));
        tokio::spawn(read(from_edge, Arc::downgrade(&pipe)));
        pipe
    }

    /// Whether Edge's end of the pipe has closed (Edge ended, or the helper did).
    pub fn is_closed(&self) -> bool {
        self.gone.is_cancelled()
    }

    /// Waits until the pipe has closed.
    pub async fn closed(&self) {
        self.gone.cancelled().await
    }

    fn id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    fn send(&self, message: &Value) {
        let mut bytes = message.to_string().into_bytes();
        bytes.push(0);
        let _ = self.to_edge.send(bytes);
    }

    /// Sends Edge the request `method` of Dots' own and returns its `result`, waiting `ASK_WAIT`.
    pub async fn ask(&self, method: &str, params: Value) -> Result<Value, HostError> {
        self.ask_within(method, params, ASK_WAIT).await
    }

    /// `ask`, waiting up to `wait`.
    pub async fn ask_within(
        &self,
        method: &str,
        params: Value,
        wait: Duration,
    ) -> Result<Value, HostError> {
        let id = self.id();
        let (tx, rx) = oneshot::channel();
        {
            let mut waiting = self.waiting.lock().unwrap();
            if waiting.ended {
                return Err(HostError::Unreachable("Edge has closed".to_string()));
            }
            waiting.own.insert(id, tx);
            self.send(&json!({ "id": id, "method": method, "params": params }));
        }
        let answer = tokio::time::timeout(wait, rx).await;
        self.waiting.lock().unwrap().own.remove(&id);
        match answer {
            Ok(Ok(answer)) => match answer.get("error") {
                Some(error) => Err(HostError::Unreachable(format!("{method}: {error}"))),
                None => Ok(answer.get("result").cloned().unwrap_or(Value::Null)),
            },
            Ok(Err(_)) => Err(HostError::Unreachable("Edge has closed".to_string())),
            Err(_) => Err(HostError::Unreachable(format!(
                "no answer to {method} within {} s",
                wait.as_secs()
            ))),
        }
    }

    /// The address of every page of this Edge (`Target.getTargets`, type `page`).
    pub async fn pages(&self) -> Result<Vec<String>, HostError> {
        let result = self.ask("Target.getTargets", json!({})).await?;
        let targets = result
            .get("targetInfos")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(page_urls(&targets))
    }

    /// Connects Playwright: what Edge sends it comes out of the receiver, under the ids it
    /// gave. A Playwright connected before is let go first (`disconnect`). `None` once the
    /// pipe has ended.
    pub fn connect(&self) -> Option<(u64, mpsc::UnboundedReceiver<String>)> {
        let mut waiting = self.waiting.lock().unwrap();
        if waiting.ended {
            return None;
        }
        if let Some(old) = waiting.client.take() {
            self.forget(old);
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let serial = self.id();
        waiting.client = Some(Client {
            serial,
            to: tx,
            ids: HashMap::new(),
            sessions: BTreeSet::new(),
        });
        Some((serial, rx))
    }

    /// A message from the Playwright connected as `serial`: sent to Edge under an id of the
    /// pipe's own. A message that is not a JSON object with a number `id`, or from a
    /// Playwright no longer connected, is dropped: false.
    pub fn relay(&self, serial: u64, text: &str) -> bool {
        let mut message = match serde_json::from_str::<Value>(text) {
            Ok(message) => message,
            Err(e) => {
                tracing::warn!("dropped a browser message from Playwright that is not JSON: {e}");
                return false;
            }
        };
        let Some(theirs) = message.get("id").filter(|id| id.is_number()).cloned() else {
            tracing::warn!("dropped a browser message from Playwright without an id");
            return false;
        };
        let mut waiting = self.waiting.lock().unwrap();
        let Some(client) = waiting.client.as_mut().filter(|c| c.serial == serial) else {
            return false;
        };
        let id = self.id();
        client.ids.insert(id, theirs);
        message["id"] = json!(id);
        self.send(&message);
        true
    }

    /// The Playwright connected as `serial` went: the sessions Edge attached for it are
    /// detached and Edge stops attaching new ones, so that the next Playwright finds every
    /// page again.
    pub fn disconnect(&self, serial: u64) {
        let mut waiting = self.waiting.lock().unwrap();
        if waiting.client.as_ref().is_some_and(|c| c.serial == serial) {
            let old = waiting.client.take().expect("a client");
            self.forget(old);
        }
    }

    /// Undoes what a Playwright set up on the pipe. The answers are not waited for.
    fn forget(&self, client: Client) {
        for session in client.sessions {
            self.send(&json!({
                "id": self.id(),
                "method": "Target.detachFromTarget",
                "params": { "sessionId": session }
            }));
        }
        self.send(&json!({
            "id": self.id(),
            "method": "Target.setAutoAttach",
            "params": { "autoAttach": false, "waitForDebuggerOnStart": false }
        }));
    }

    /// One message from Edge: an answer to Dots or to Playwright, or an event for Playwright.
    fn dispatch(&self, frame: &[u8]) {
        let mut message = match serde_json::from_slice::<Value>(frame) {
            Ok(message @ Value::Object(_)) => message,
            _ => {
                tracing::warn!("dropped a browser message from Edge that is not a JSON object");
                return;
            }
        };
        let mut waiting = self.waiting.lock().unwrap();
        if let Some(id) = message.get("id") {
            let id = id.as_u64();
            if let Some(own) = id.and_then(|id| waiting.own.remove(&id)) {
                let _ = own.send(message);
                return;
            }
            let Some(client) = waiting.client.as_mut() else {
                return;
            };
            let Some(theirs) = id.and_then(|id| client.ids.remove(&id)) else {
                return;
            };
            message["id"] = theirs;
            let _ = client.to.send(message.to_string());
            return;
        }
        let Some(client) = waiting.client.as_mut() else {
            return;
        };
        if message.get("sessionId").is_none() {
            let method = message.get("method").and_then(Value::as_str);
            let session = message
                .pointer("/params/sessionId")
                .and_then(Value::as_str)
                .map(str::to_string);
            match (method, session) {
                (Some("Target.attachedToTarget"), Some(session)) => {
                    client.sessions.insert(session);
                }
                (Some("Target.detachedFromTarget"), Some(session)) => {
                    client.sessions.remove(&session);
                }
                _ => {}
            }
        }
        let _ = client.to.send(String::from_utf8_lossy(frame).into_owned());
    }

    /// The pipe ended: every request waiting fails, and Playwright's connection ends.
    fn end(&self) {
        self.gone.cancel();
        let mut waiting = self.waiting.lock().unwrap();
        waiting.ended = true;
        waiting.own.clear();
        waiting.client = None;
    }
}

async fn read<R: AsyncRead + Unpin>(mut from_edge: R, pipe: Weak<Pipe>) {
    let mut frames = Frames::default();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match from_edge.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let Some(pipe) = pipe.upgrade() else {
            return;
        };
        for frame in frames.push(&buf[..n]) {
            pipe.dispatch(&frame);
        }
    }
    if let Some(pipe) = pipe.upgrade() {
        pipe.end();
    }
}

async fn write<W: AsyncWrite + Unpin>(
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
    mut to_edge: W,
    pipe: Weak<Pipe>,
) {
    while let Some(bytes) = rx.recv().await {
        if to_edge.write_all(&bytes).await.is_err() || to_edge.flush().await.is_err() {
            if let Some(pipe) = pipe.upgrade() {
                pipe.end();
            }
            return;
        }
    }
    // Every sender is gone: Edge's input closes, and the helper closes Edge.
    let _ = to_edge.shutdown().await;
}
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test browser_pipe_test`
Expected: PASS (5 tests).

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo clippy --workspace --all-targets`, `cargo fmt`, `cargo fmt --check`.
Expected: all pass but the known failures; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/browser/mod.rs crates/dots-core/src/browser/pipe.rs crates/dots-core/tests/common/edge_pipe.rs crates/dots-core/tests/common/mod.rs crates/dots-core/tests/browser_pipe_test.rs
git commit -m "feat(core): a dot's Edge over its debugging pipe, with Dots' and Playwright's ids kept apart"
```

### Task 2: The door: one connection per secret

`door.rs` holds the doors of the runs' Playwright servers: `Doors::open` gives a pipe a fresh 128-bit secret; the first connection with it is relayed to the pipe (`connect`, the WebSocket handler for `GET /cdp/{secret}`), and the door is spent when that connection ends. A second connection, a wrong or a spent secret is 404 and logged. axum gains its `ws` feature; the tests connect with `tokio-tungstenite` as Playwright does, to a server of the test's own with the door's route, and play Edge in memory.

**Files:**

- Modify: `crates/dots-core/Cargo.toml` (axum's `ws` feature; two dev-dependencies), `Cargo.lock` (by cargo)
- Create: `crates/dots-core/src/browser/door.rs`
- Modify: `crates/dots-core/src/browser/mod.rs` (the module list)
- Test: `crates/dots-core/tests/browser_door_test.rs` (new)

**Interfaces:**

- Consumes: `pipe::{Pipe, MAX_MESSAGE}` (Task 1); `util::ct_eq`; `getrandom`, `base64` (dependencies already); axum's `WebSocketUpgrade`, `WebSocket`, `Message`, `WebSocketUpgradeRejection`.
- Produces (in `dots_core::browser::door`):
  - `pub fn new_secret() -> String` (16 random bytes, base64url, 22 characters).
  - `#[derive(Default)] pub struct Doors` with `fn open(&self, pipe: Arc<Pipe>) -> String`, `fn spent(&self, secret: &str) -> bool`, `fn shut(&self, secret: &str)`.
  - `pub async fn connect(doors: Arc<Doors>, secret: String, upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>) -> Response`.

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/Cargo.toml`, replace:**

```toml
axum = "0.8"
```

**with:**

```toml
axum = { version = "0.8", features = ["ws"] }
```

**In `crates/dots-core/Cargo.toml`, replace:**

```toml
[dev-dependencies]
tempfile = "3"
wiremock = "0.6"
```

**with:**

```toml
[dev-dependencies]
futures-util = "0.3"
tempfile = "3"
tokio-tungstenite = "0.29"
wiremock = "0.6"
```

**Create `crates/dots-core/tests/browser_door_test.rs`:**

```rust
//! The door of a run's Playwright to its dot's Edge (browser spec section 14), on a server of
//! the test's own with the door's route, and the test playing Edge: one connection per secret
//! (two clients racing, a reconnection, a wrong secret, a request that is not a WebSocket), the
//! relay both ways with large messages, what Playwright set up undone when it goes, and Edge
//! closing mid-relay.

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::Path;
use axum::routing::get;
use axum::Router;
use common::edge_pipe::TestEdge;
use dots_core::browser::door::{self, new_secret, Doors};
use dots_core::browser::pipe::MAX_MESSAGE;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

/// A server with the door's route on `listener`, as Dots' own server has it.
fn serve_on(listener: tokio::net::TcpListener, doors: Arc<Doors>) {
    let app = Router::new().route(
        "/cdp/{secret}",
        get(
            move |Path(secret): Path<String>,
                  upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>| {
                door::connect(doors.clone(), secret, upgrade)
            },
        ),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
}

/// `serve_on` a port of its own; its port.
async fn serve(doors: Arc<Doors>) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    serve_on(listener, doors);
    port
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Connects as Playwright does, taking messages of up to 256 MB.
async fn enter(port: u16, secret: &str) -> Result<Socket, u16> {
    let url = format!("ws://127.0.0.1:{port}/cdp/{secret}");
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    match tokio_tungstenite::connect_async_with_config(url, Some(config), true).await {
        Ok((socket, _)) => Ok(socket),
        Err(WsError::Http(answer)) => Err(answer.status().as_u16()),
        Err(e) => panic!("{e}"),
    }
}

/// The next message from Edge as Playwright reads it.
async fn next(socket: &mut Socket) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("a message within 10 s")
            .expect("the socket is open")
            .unwrap();
        if let Message::Text(text) = message {
            return serde_json::from_str(text.as_str()).unwrap();
        }
    }
}

async fn send(socket: &mut Socket, message: Value) {
    socket
        .send(Message::Text(message.to_string().into()))
        .await
        .unwrap();
}

/// Whether the door of `secret` is spent within 5 s.
async fn spent_within(doors: &Doors, secret: &str) -> bool {
    for _ in 0..250 {
        if doors.spent(secret) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[test]
fn a_secret_is_128_random_bits() {
    let a = new_secret();
    assert_eq!(a.len(), 22);
    assert!(a
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    assert_ne!(a, new_secret());
}

#[tokio::test]
async fn a_secret_lets_one_connection_in_once() {
    let edge = TestEdge::start();
    let doors = Arc::new(Doors::default());
    let port = serve(doors.clone()).await;
    let secret = doors.open(edge.pipe.clone());
    // Two clients race for one secret: exactly one comes in.
    let (a, b) = tokio::join!(enter(port, &secret), enter(port, &secret));
    let (mut inside, refused) = match (a, b) {
        (Ok(socket), Err(status)) | (Err(status), Ok(socket)) => (socket, status),
        (a, b) => panic!("one of two, not {:?} and {:?}", a.is_ok(), b.is_ok()),
    };
    assert_eq!(refused, 404);
    assert!(!doors.spent(&secret), "in use, not spent");
    assert_eq!(enter(port, "not-the-secret").await.err(), Some(404));
    // Playwright reconnects after its connection dropped: the secret is spent.
    inside.close(None).await.unwrap();
    assert!(spent_within(&doors, &secret).await);
    assert_eq!(enter(port, &secret).await.err(), Some(404));
    doors.shut(&secret);
    assert!(!doors.spent(&secret));
    assert_eq!(enter(port, &secret).await.err(), Some(404));
    // A request that is not a WebSocket spends the door too.
    let plain = doors.open(edge.pipe.clone());
    let answer = reqwest::get(format!("http://127.0.0.1:{port}/cdp/{plain}"))
        .await
        .unwrap();
    assert!(answer.status().is_client_error());
    assert!(doors.spent(&plain));
    assert_eq!(enter(port, &plain).await.err(), Some(404));
}

#[tokio::test]
async fn playwright_and_edge_talk_through_the_door() {
    let mut edge = TestEdge::start();
    let doors = Arc::new(Doors::default());
    let port = serve(doors.clone()).await;
    let secret = doors.open(edge.pipe.clone());
    let mut playwright = enter(port, &secret).await.unwrap();
    send(
        &mut playwright,
        json!({"id": 1, "method": "Target.setAutoAttach",
               "params": {"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true}}),
    )
    .await;
    let request = edge.next().await;
    assert_eq!(request["method"], "Target.setAutoAttach");
    edge.send(&json!({"method": "Target.attachedToTarget",
                      "params": {"sessionId": "S1", "targetInfo": {"type": "page"}}}))
        .await;
    edge.send(&json!({"id": request["id"], "result": {}})).await;
    assert_eq!(next(&mut playwright).await["params"]["sessionId"], "S1");
    assert_eq!(next(&mut playwright).await, json!({"id": 1, "result": {}}));
    // Large messages pass both ways: 20 MB from Playwright, 30 MB from Edge.
    let text = "y".repeat(20_000_000);
    send(
        &mut playwright,
        json!({"id": 2, "sessionId": "S1", "method": "Runtime.evaluate",
               "params": {"expression": text}}),
    )
    .await;
    let evaluate = edge.next().await;
    assert_eq!(
        evaluate["params"]["expression"].as_str().unwrap().len(),
        20_000_000
    );
    edge.send(&json!({"id": evaluate["id"], "sessionId": "S1",
                      "result": {"data": "z".repeat(30_000_000)}}))
        .await;
    let answer = next(&mut playwright).await;
    assert_eq!(answer["id"], 2);
    assert_eq!(answer["result"]["data"].as_str().unwrap().len(), 30_000_000);
    // Playwright goes: the session Edge attached for it is detached.
    playwright.close(None).await.unwrap();
    let detach = edge.next().await;
    assert_eq!(detach["method"], "Target.detachFromTarget");
    assert_eq!(detach["params"]["sessionId"], "S1");
    assert_eq!(edge.next().await["params"]["autoAttach"], false);
    assert!(spent_within(&doors, &secret).await);
}

#[tokio::test]
async fn edge_closing_mid_relay_ends_the_connection() {
    let mut edge = TestEdge::start();
    let doors = Arc::new(Doors::default());
    let port = serve(doors.clone()).await;
    let secret = doors.open(edge.pipe.clone());
    let mut playwright = enter(port, &secret).await.unwrap();
    send(&mut playwright, json!({"id": 1, "method": "Page.navigate"})).await;
    edge.next().await;
    edge.close().await;
    let end = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match playwright.next().await {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(end.is_ok(), "the socket closes with Edge");
    assert!(spent_within(&doors, &secret).await);
    // A door to an Edge that has gone lets nobody in for long.
    let late = doors.open(edge.pipe.clone());
    if let Ok(mut socket) = enter(port, &late).await {
        let closed = tokio::time::timeout(Duration::from_secs(10), socket.next()).await;
        assert!(matches!(
            closed,
            Ok(Some(Ok(Message::Close(_))) | None | Some(Err(_)))
        ));
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test browser_door_test`
Expected: cargo adds `tokio-tungstenite 0.29.0`, `tungstenite 0.29.0` and `data-encoding` to `Cargo.lock` (fetched from crates.io the first time); then FAIL to compile: unresolved import `dots_core::browser::door`.

- [ ] **Step 3: Add the door**

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
pub mod gate;
pub mod host;
```

**with:**

```rust
pub mod door;
pub mod gate;
pub mod host;
```

**Create `crates/dots-core/src/browser/door.rs`:**

```rust
//! The door through which a run's Playwright reaches its dot's Edge (browser spec section 14):
//! `ws://127.0.0.1:<dots port>/cdp/<secret>` on Dots' own server. Each Playwright server gets a
//! fresh secret when it starts. The first connection with it is relayed to the Edge's pipe
//! (`pipe.rs`); every other one (a second connection, a wrong or a spent secret) is refused
//! with 404 and logged.

use std::sync::{Arc, Mutex};

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;

use super::pipe::{Pipe, MAX_MESSAGE};
use crate::util::ct_eq;

/// A new secret: 128 random bits, base64url without padding (22 characters).
pub fn new_secret() -> String {
    let mut buf = [0u8; 16];
    getrandom::fill(&mut buf).expect("OS random number generator unavailable");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

enum Door {
    /// No one has come in yet.
    Waiting(Arc<Pipe>),
    /// Playwright is connected.
    Open,
    /// The connection was made and has ended, or the upgrade failed.
    Spent,
}

/// The open doors, by secret.
#[derive(Default)]
pub struct Doors {
    doors: Mutex<Vec<(String, Door)>>,
}

impl Doors {
    /// A new door to `pipe`; returns its secret.
    pub fn open(&self, pipe: Arc<Pipe>) -> String {
        let secret = new_secret();
        self.doors
            .lock()
            .unwrap()
            .push((secret.clone(), Door::Waiting(pipe)));
        secret
    }

    /// Whether the door of `secret` was used and its connection has ended: the Playwright
    /// server behind it cannot come in again.
    pub fn spent(&self, secret: &str) -> bool {
        self.doors
            .lock()
            .unwrap()
            .iter()
            .any(|(s, door)| ct_eq(s, secret) && matches!(door, Door::Spent))
    }

    /// Forgets the door of `secret` (its Playwright server stopped).
    pub fn shut(&self, secret: &str) {
        self.doors
            .lock()
            .unwrap()
            .retain(|(s, _)| !ct_eq(s, secret));
    }

    /// The pipe behind the door of `secret`, once: the door is open from then on.
    fn enter(&self, secret: &str) -> Option<Arc<Pipe>> {
        let mut doors = self.doors.lock().unwrap();
        let (_, door) = doors.iter_mut().find(|(s, _)| ct_eq(s, secret))?;
        match std::mem::replace(door, Door::Open) {
            Door::Waiting(pipe) => Some(pipe),
            other => {
                *door = other;
                None
            }
        }
    }

    /// The connection through the door of `secret` ended.
    fn leave(&self, secret: &str) {
        if let Some((_, door)) = self
            .doors
            .lock()
            .unwrap()
            .iter_mut()
            .find(|(s, _)| ct_eq(s, secret))
        {
            *door = Door::Spent;
        }
    }
}

/// `GET /cdp/<secret>`: Playwright's WebSocket to the Edge behind the door of `secret`, which
/// is spent by it. Anything else is 404.
pub async fn connect(
    doors: Arc<Doors>,
    secret: String,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let Some(pipe) = doors.enter(&secret) else {
        tracing::warn!("refused a connection to a dot's browser: its secret is wrong or spent");
        return StatusCode::NOT_FOUND.into_response();
    };
    let upgrade = match upgrade {
        Ok(upgrade) => upgrade,
        Err(rejection) => {
            tracing::warn!("refused a connection to a dot's browser: {rejection}");
            doors.leave(&secret);
            return rejection.into_response();
        }
    };
    let failed = (doors.clone(), secret.clone());
    upgrade
        .max_message_size(MAX_MESSAGE)
        .max_frame_size(MAX_MESSAGE)
        .on_failed_upgrade(move |e| {
            tracing::warn!("a connection to a dot's browser failed: {e}");
            failed.0.leave(&failed.1);
        })
        .on_upgrade(move |socket| async move {
            relay(&pipe, socket).await;
            doors.leave(&secret);
        })
}

/// Relays between Playwright's WebSocket and the Edge's pipe until either ends.
async fn relay(pipe: &Pipe, mut socket: WebSocket) {
    let Some((serial, mut from_edge)) = pipe.connect() else {
        let _ = socket.send(Message::Close(None)).await;
        return;
    };
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    pipe.relay(serial, text.as_str());
                }
                Some(Ok(Message::Binary(bytes))) => match std::str::from_utf8(&bytes) {
                    Ok(text) => {
                        pipe.relay(serial, text);
                    }
                    Err(_) => tracing::warn!("dropped a browser message from Playwright that is not UTF-8"),
                },
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
            },
            outgoing = from_edge.recv() => match outgoing {
                Some(text) => {
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                // Edge closed, or another Playwright took the pipe.
                None => {
                    let _ = socket.send(Message::Close(None)).await;
                    break;
                }
            },
        }
    }
    pipe.disconnect(serial);
}
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test browser_door_test`, then `cargo test -p dots-core --test browser_pipe_test`
Expected: PASS (4 tests; 5 tests).

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo clippy --workspace --all-targets`, `cargo fmt`, `cargo fmt --check`.
Expected: all pass but the known failures; no warnings.

- [ ] **Step 6: Commit**

```bash
git add Cargo.lock crates/dots-core/Cargo.toml crates/dots-core/src/browser/mod.rs crates/dots-core/src/browser/door.rs crates/dots-core/tests/browser_door_test.rs
git commit -m "feat(core): a door for Playwright to a dot's Edge, one connection per secret"
```

### Task 3: Edge on its pipe, and Playwright through its door

`host.rs` starts a dot's Edge through `browser-pipe.mjs` under `node` (or, in tests, `fake-browser` directly) and keeps its `Pipe`: no port, no port file. `open` asks `Browser.getVersion` to know Edge is up; `close` sends `Browser.close`; a closed pipe means the window was closed, and the next `open` starts Edge again; when Dots stops, the helper's stdin ends and it closes Edge. `RunBrowser` opens a door for each Playwright server it starts, names it in `PLAYWRIGHT_MCP_CDP_ENDPOINT`, and reads the pages over the pipe. Dots' server serves the doors at `/cdp/{secret}`. `fake-browser` speaks the pipe, on stdin and stdout or, under the helper, on descriptors 3 and 4.

**Files:**

- Create (replacing the file): `crates/dots-core/src/bin/fake-browser.rs`
- Modify: `crates/dots-core/src/bin/fake-mcp.rs` (its log records `PLAYWRIGHT_MCP_*` variables)
- Modify: `crates/dots-core/tests/common/fake_browser.rs` (`host`, `control`, `received`, `methods`)
- Create (replacing the file): `crates/dots-core/tests/browser_host_test.rs`
- Modify: `crates/dots-core/tests/browser_call_test.rs` (imports, `DOTS_PORT`, the hosts, the Playwright command's arguments)
- Modify: `crates/dots-core/tests/local_browser_test.rs`, `crates/dots-core/tests/claude_browser_test.rs` (the host and the port; one test appended to the second), `crates/dots-core/tests/runtime_browser_test.rs` (the helper's switches)
- Create: `crates/dots-core/src/browser/browser-pipe.mjs`
- Create (replacing the file): `crates/dots-core/src/browser/host.rs`
- Modify: `crates/dots-core/src/browser/mod.rs` (imports, `CDP_ENDPOINT_VAR`, `args_for`, `Browsers`, `Live`, `RunBrowser::{ready, stop_server, start_server, page_now, close}`)
- Modify: `crates/dots-core/src/server/mod.rs` (one route), `crates/dots-core/src/server/browser.rs` (imports; `door` appended), `crates/dots-core/src/runtime.rs` (one argument)

**Interfaces:**

- Consumes: `pipe::Pipe` (Task 1), `door::{Doors, connect}` (Task 2); `proc::hide_window`, `mcp::resolve_program`; `Runner::browsers`.
- Produces:
  - In `dots_core::browser::host`: `PIPE_FLAGS: [&str; 2]`, `HELPER: &str`, `HELPER_FILE = "browser-pipe.mjs"`, `START_WAIT` (20 s), `CLOSE_WAIT` (5 s); `HostError::NoNode` added; `#[derive(Debug, Clone)] pub struct Opened { pub edge: Arc<Pipe>, pub started: bool }`; `BrowserHost` loses `pages(port)`; `#[derive(Debug, Clone, PartialEq, Eq)] pub enum Launcher { Node(String), Direct }`; `EdgeHost::new(root)` (the helper under `node`) and `EdgeHost::with_launcher(root, launcher)`; `pub fn edge_args(profile: &Path) -> Vec<String>` (no port).
  - In `dots_core::browser`: `pub const CDP_ENDPOINT_VAR: &str = "PLAYWRIGHT_MCP_CDP_ENDPOINT"`; `PlaywrightCommand::args_for(&self, output: &Path) -> Vec<String>`; `Browsers::new(store, host, playwright, port: u16)`; `Browsers::doors(&self) -> &Arc<Doors>`.
  - In the server: `GET /cdp/{secret}` (`server::browser::door`).
  - Tests: `common::fake_browser::host(root) -> EdgeHost`; `FakeEdges::{control, received, methods}` (`port` is gone).

- [ ] **Step 1: Write the fake Edge on its pipe and the failing tests**

**Create `crates/dots-core/src/bin/fake-browser.rs`:**

```rust
//! Test stand-in for Microsoft Edge in dots-core tests: `fake-browser --user-data-dir=<dir>
//! [other switches] [address]`.
//!
//! - It holds `<dir>/lockfile` as Edge does: while it runs, no other process can open that
//!   file for writing. A start on a profile another process holds exits 0 at once without
//!   serving, as Edge hands over to the Edge that holds it.
//! - It speaks Edge's debugging pipe: JSON messages, each ended by a NUL byte, read from file
//!   descriptor 3 and written to 4 when it was started with `--remote-debugging-pipe` (by the
//!   helper `browser-pipe.mjs`), else read from stdin and written to stdout (`EdgeHost` with
//!   `Launcher::Direct`). It exits when its input ends, as Edge does when the helper closes it.
//! - It answers `Browser.getVersion`; `Target.getTargets` (its pages, most recent first, then
//!   a `service_worker` target, as Edge lists an extension's); `Target.setAutoAttach` with
//!   `autoAttach: true` (first an event `Target.attachedToTarget` per page, session
//!   `S<page id>`); `Browser.close` (answers, then exits); `Fake.event` (first an event
//!   `Fake.event` with the same params); `Fake.big` with `size` (a result `{"data": "<size
//!   x's>"}`); any other request with an empty result. An answer carries its request's
//!   `sessionId`. Each message goes out in two pieces, as a pipe may deliver it.
//! - Every message it reads is appended to `<dir>/fake-cdp.jsonl`; one that is not JSON as
//!   `{"invalid": "<text>"}`, unanswered.
//! - Test controls, over HTTP on a port of its own on 127.0.0.1 that Dots never uses: `POST
//!   /fake/pages` with a JSON array of addresses makes those its pages, most recent first;
//!   `POST /fake/quit` exits at once, as when the person closes the window.
//! - Its first pages are `<dir>/fake-pages.json` (a JSON array) when that file exists, else the
//!   address it was given, else `about:blank`.
//! - Each start appends `{"args": [...], "control": <port>, "pid": <pid>}` to
//!   `<dir>/fake-browser.jsonl`. It exits when `<dir>` is gone, and after 120 s.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

type Output = Mutex<Box<dyn Write + Send>>;

struct Fake {
    next: AtomicU64,
    /// `(id, address)`, most recent first.
    pages: Mutex<Vec<(String, String)>>,
    received: Mutex<File>,
}

impl Fake {
    fn page(&self, url: String) -> (String, String) {
        (
            format!("P{}", self.next.fetch_add(1, Ordering::Relaxed)),
            url,
        )
    }

    fn record(&self, message: &Value) {
        let _ = writeln!(self.received.lock().unwrap(), "{message}");
    }

    fn pages(&self) -> Vec<(String, String)> {
        self.pages.lock().unwrap().clone()
    }
}

/// The value of `--name=value`.
fn switch(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .find_map(|a| a.strip_prefix(name)?.strip_prefix('=').map(str::to_string))
}

/// Holds `<dir>/lockfile` as Edge does; `None` when another process holds it.
fn hold_lock(dir: &Path) -> Option<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ: others may read the file, not open it for writing.
        options.share_mode(1);
    }
    let file = options.open(dir.join("lockfile")).ok()?;
    #[cfg(not(windows))]
    file.try_lock().ok()?;
    Some(file)
}

/// The pipe's ends: descriptors 3 and 4 when the helper started it, else stdin and stdout.
fn pipe_ends(args: &[String]) -> (Box<dyn Read + Send>, Box<dyn Write + Send>) {
    if args.iter().any(|a| a == "--remote-debugging-pipe") {
        let (input, output) = inherited();
        (Box::new(input), Box::new(output))
    } else {
        (Box::new(std::io::stdin()), Box::new(std::io::stdout()))
    }
}

#[cfg(windows)]
fn inherited() -> (File, File) {
    use std::os::windows::io::{FromRawHandle, RawHandle};
    extern "C" {
        fn _get_osfhandle(fd: i32) -> isize;
    }
    // SAFETY: the helper gave this process the pipe's ends as its C descriptors 3 and 4, and
    // each handle is owned once, here.
    unsafe {
        (
            File::from_raw_handle(_get_osfhandle(3) as RawHandle),
            File::from_raw_handle(_get_osfhandle(4) as RawHandle),
        )
    }
}

#[cfg(not(windows))]
fn inherited() -> (File, File) {
    use std::os::fd::FromRawFd;
    // SAFETY: the helper gave this process the pipe's ends as its descriptors 3 and 4, and
    // each is owned once, here.
    unsafe { (File::from_raw_fd(3), File::from_raw_fd(4)) }
}

/// One message, ended by its NUL byte, in two pieces.
fn send(output: &Output, message: &Value) {
    let mut bytes = message.to_string().into_bytes();
    bytes.push(0);
    let (first, second) = bytes.split_at(bytes.len() / 2);
    let mut output = output.lock().unwrap();
    let _ = output.write_all(first);
    let _ = output.flush();
    std::thread::sleep(Duration::from_millis(1));
    let _ = output.write_all(second);
    let _ = output.flush();
}

fn answer(fake: &Fake, frame: &[u8], output: &Output) {
    let Ok(message) = serde_json::from_slice::<Value>(frame) else {
        fake.record(&json!({"invalid": String::from_utf8_lossy(frame)}));
        return;
    };
    fake.record(&message);
    let Some(id) = message.get("id") else {
        return;
    };
    let method = message["method"].as_str().unwrap_or_default();
    let params = &message["params"];
    let mut result = json!({});
    match method {
        "Browser.getVersion" => {
            result = json!({"product": "Edg/0.0.0.0 (fake)", "protocolVersion": "1.3"});
        }
        "Target.getTargets" => {
            let mut targets: Vec<Value> = fake
                .pages()
                .into_iter()
                .map(|(id, url)| json!({"targetId": id, "type": "page", "title": "", "url": url, "attached": false}))
                .collect();
            targets.push(
                json!({"targetId": "SW", "type": "service_worker", "title": "Service Worker",
                                "url": "chrome-extension://fake/background.js", "attached": false}),
            );
            result = json!({"targetInfos": targets});
        }
        "Target.setAutoAttach" if params["autoAttach"] == true => {
            for (id, url) in fake.pages() {
                send(
                    output,
                    &json!({"method": "Target.attachedToTarget", "params": {
                        "sessionId": format!("S{id}"),
                        "targetInfo": {"targetId": id, "type": "page", "url": url, "attached": true},
                        "waitingForDebugger": false
                    }}),
                );
            }
        }
        "Fake.event" => send(output, &json!({"method": "Fake.event", "params": params})),
        "Fake.big" => {
            let size = params["size"].as_u64().unwrap_or(0) as usize;
            result = json!({"data": "x".repeat(size)});
        }
        _ => {}
    }
    let mut reply = json!({"id": id, "result": result});
    if let Some(session) = message.get("sessionId") {
        reply["sessionId"] = session.clone();
    }
    send(output, &reply);
    if method == "Browser.close" {
        std::process::exit(0);
    }
}

/// Reads the pipe until its input ends, then exits.
fn serve_pipe(fake: Arc<Fake>, mut input: Box<dyn Read + Send>, output: Output) {
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match input.read(&mut buf) {
            Ok(0) | Err(_) => std::process::exit(0),
            Ok(n) => n,
        };
        pending.extend_from_slice(&buf[..n]);
        while let Some(end) = pending.iter().position(|b| *b == 0) {
            let frame: Vec<u8> = pending.drain(..=end).collect();
            if end > 0 {
                answer(&fake, &frame[..end], &output);
            }
        }
    }
}

fn exit_soon() {
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        std::process::exit(0);
    });
}

async fn set_pages(State(fake): State<Arc<Fake>>, Json(urls): Json<Vec<String>>) -> StatusCode {
    let pages: Vec<(String, String)> = urls.into_iter().map(|url| fake.page(url)).collect();
    *fake.pages.lock().unwrap() = pages;
    StatusCode::OK
}

async fn quit() -> StatusCode {
    exit_soon();
    StatusCode::OK
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(switch(&args, "--user-data-dir").expect("--user-data-dir"));
    std::fs::create_dir_all(&dir).unwrap();
    // Edge hands over to the Edge that holds the profile, and exits.
    let Some(_lock) = hold_lock(&dir) else {
        return;
    };
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let control = listener.local_addr().unwrap().port();
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("fake-browser.jsonl"))
        .unwrap();
    writeln!(
        log,
        "{}",
        json!({"args": args, "control": control, "pid": std::process::id()})
    )
    .unwrap();
    let first: Vec<String> = std::fs::read_to_string(dir.join("fake-pages.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| {
            vec![args
                .iter()
                .rev()
                .find(|a| !a.starts_with("--"))
                .cloned()
                .unwrap_or_else(|| "about:blank".to_string())]
        });
    let received = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("fake-cdp.jsonl"))
        .unwrap();
    let fake = Arc::new(Fake {
        next: AtomicU64::new(0),
        pages: Mutex::new(Vec::new()),
        received: Mutex::new(received),
    });
    let pages: Vec<(String, String)> = first.into_iter().map(|url| fake.page(url)).collect();
    *fake.pages.lock().unwrap() = pages;
    // Gone with its test: when the profile's folder is removed, or after 120 s.
    let watched = dir.clone();
    tokio::spawn(async move {
        let end = Instant::now() + Duration::from_secs(120);
        while Instant::now() < end && watched.exists() {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        std::process::exit(0);
    });
    let (input, output) = pipe_ends(&args);
    let piped = fake.clone();
    std::thread::spawn(move || serve_pipe(piped, input, Mutex::new(output)));
    let app = Router::new()
        .route("/fake/pages", post(set_pages))
        .route("/fake/quit", post(quit))
        .with_state(fake);
    axum::serve(listener, app).await.unwrap();
}
```

**In `crates/dots-core/tests/common/fake_browser.rs`, replace:**

```rust
//! The test stand-in for Microsoft Edge, `fake-browser` (`src/bin/fake-browser.rs`): its path,
//! for `Settings.edge_path` or `EdgeHost::open`; what each start on a dot's profile recorded;
//! and its controls. Every fake running on a profile under `root` is told to quit when this is
//! dropped, so a failed test leaves no Edge behind.
```

**with:**

```rust
//! The test stand-in for Microsoft Edge, `fake-browser` (`src/bin/fake-browser.rs`): its path,
//! for `Settings.edge_path` or `EdgeHost::open`; a host that starts it without the helper; what
//! each start on a dot's profile recorded, and what it read on its pipe; and its controls.
//! Every fake running on a profile under `root` is told to quit when this is dropped, so a
//! failed test leaves no Edge behind.
```

**In `crates/dots-core/tests/common/fake_browser.rs`, replace:**

```rust
use dots_core::browser::PlaywrightCommand;
```

**with:**

```rust
use dots_core::browser::host::{EdgeHost, Launcher};
use dots_core::browser::PlaywrightCommand;
```

**In `crates/dots-core/tests/common/fake_browser.rs`, replace:**

```rust
/// The program `fake-browser`.
pub fn exe() -> String {
    env!("CARGO_BIN_EXE_fake-browser").to_string()
}
```

**with:**

```rust
/// The program `fake-browser`.
pub fn exe() -> String {
    env!("CARGO_BIN_EXE_fake-browser").to_string()
}

/// A host that starts `fake-browser` on its stdin and stdout, without the helper and Node.js.
pub fn host(root: impl Into<PathBuf>) -> EdgeHost {
    EdgeHost::with_launcher(root, Launcher::Direct)
}
```

**In `crates/dots-core/tests/common/fake_browser.rs`, replace:**

```rust
    /// Every start on the dot's profile, oldest first: `{"args": [...], "port": n, "pid": n}`.
```

**with:**

```rust
    /// Every start on the dot's profile, oldest first: `{"args": [...], "control": n, "pid":
    /// n}`.
```

**In `crates/dots-core/tests/common/fake_browser.rs`, replace:**

```rust
    /// The port of the newest start.
    pub fn port(&self, dot_id: &str) -> Option<u16> {
        self.starts(dot_id)
            .last()
            .and_then(|start| start["port"].as_u64())
            .map(|port| port as u16)
    }
```

**with:**

```rust
    /// The control port of the newest start.
    pub fn control(&self, dot_id: &str) -> Option<u16> {
        self.starts(dot_id)
            .last()
            .and_then(|start| start["control"].as_u64())
            .map(|port| port as u16)
    }

    /// Every message the fakes on the dot's profile read on their pipes, oldest first.
    pub fn received(&self, dot_id: &str) -> Vec<Value> {
        match std::fs::read_to_string(self.profile(dot_id).join("fake-cdp.jsonl")) {
            Ok(text) => text
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// The methods of `received`, in order.
    pub fn methods(&self, dot_id: &str) -> Vec<String> {
        self.received(dot_id)
            .iter()
            .map(|m| m["method"].as_str().unwrap_or_default().to_string())
            .collect()
    }
```

**In `crates/dots-core/tests/common/fake_browser.rs`, replace:**

```rust
        let port = self.port(dot_id).expect("a started fake");
```

**with:**

```rust
        let port = self.control(dot_id).expect("a started fake");
```

**In `crates/dots-core/tests/common/fake_browser.rs`, replace:**

```rust
        if let Some(port) = self.port(dot_id) {
```

**with:**

```rust
        if let Some(port) = self.control(dot_id) {
```

**Create `crates/dots-core/tests/browser_host_test.rs`:**

```rust
//! Microsoft Edge for dots (browser spec sections 2.1 and 14) through `EdgeHost`, with
//! `fake-browser` in Edge's place: the switches and profile of a start, with no port; a second
//! open that finds it; a Dots that starts again starting a new one; the pages it lists and
//! closing it, over its pipe; a window the person closed; a profile another Edge holds; an Edge
//! or a Node.js that is not there; the helper `browser-pipe.mjs`, which gives Edge its pipe and
//! closes it when Dots goes; and moving downloads.

mod common;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use common::fake_browser::{exe, host, FakeEdges};
use common::playwright::JSON_LIST;
use dots_core::browser::host::{
    edge_args, edge_candidates, find_edge, move_downloads, page_urls, BrowserHost, EdgeHost,
    HostError, Launcher, EDGE_FLAGS, HELPER, HELPER_FILE, PIPE_FLAGS,
};
use dots_core::browser::NO_NODE;
use serde_json::Value;

/// Dropped in this order: the fakes quit, then the folder goes.
struct Env {
    edges: FakeEdges,
    host: EdgeHost,
    root: PathBuf,
    dir: tempfile::TempDir,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("browser");
    Env {
        edges: FakeEdges::new(&root),
        host: host(&root),
        root,
        dir,
    }
}

fn edge() -> PathBuf {
    PathBuf::from(exe())
}

#[tokio::test]
async fn edge_starts_on_the_dots_profile_with_its_switches_and_no_port() {
    let e = env();
    let opened = e.host.open("D1", &edge()).await.unwrap();
    assert!(opened.started);
    let starts = e.edges.starts("D1");
    assert_eq!(starts.len(), 1);
    let args: Vec<String> = serde_json::from_value(starts[0]["args"].clone()).unwrap();
    assert_eq!(args, edge_args(&e.host.profile("D1")));
    assert_eq!(
        args[0],
        format!(
            "--user-data-dir={}",
            e.root.join("D1").join("profile").display()
        )
    );
    assert_eq!(args[1..8], EDGE_FLAGS);
    assert_eq!(args[8], "about:blank");
    assert_eq!(args.len(), 9);
    assert!(
        args.iter().all(|a| !a.contains("remote-debugging")),
        "{args:?}"
    );
    // Downloads go to the dot's own folder, without a question.
    let prefs: Value = serde_json::from_str(
        &std::fs::read_to_string(e.host.profile("D1").join("Default").join("Preferences")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        prefs["download"]["default_directory"].as_str(),
        Some(e.host.downloads("D1").to_str().unwrap())
    );
    assert_eq!(prefs["download"]["prompt_for_download"], false);
    assert!(e.host.downloads("D1").is_dir());
    // Its pages, read over its pipe: the blank page; the extension's service worker is not one.
    assert_eq!(opened.edge.pages().await.unwrap(), ["about:blank"]);
    assert_eq!(
        e.edges.methods("D1"),
        ["Browser.getVersion", "Target.getTargets"]
    );
}

#[tokio::test]
async fn a_second_open_finds_it_and_a_dots_that_starts_again_starts_another() {
    let e = env();
    let first = e.host.open("D1", &edge()).await.unwrap();
    let again = e.host.open("D1", &edge()).await.unwrap();
    assert!(!again.started);
    assert!(Arc::ptr_eq(&first.edge, &again.edge));
    assert_eq!(e.edges.starts("D1").len(), 1);
    // Another dot gets an Edge of its own.
    let other = e.host.open("D2", &edge()).await.unwrap();
    assert!(!Arc::ptr_eq(&other.edge, &first.edge));
    assert!(e.edges.running("D1") && e.edges.running("D2"));
    // Dots stops: its Edges close with it. A Dots that starts again starts a new one.
    drop((e.host, first, again, other));
    assert!(e.edges.gone_within("D1", Duration::from_secs(10)).await);
    assert!(e.edges.gone_within("D2", Duration::from_secs(10)).await);
    let restarted = host(&e.root);
    assert!(restarted.open("D1", &edge()).await.unwrap().started);
    assert_eq!(e.edges.starts("D1").len(), 2);
}

#[tokio::test]
async fn closing_ends_it_over_its_pipe_and_a_closed_window_starts_again() {
    let e = env();
    let opened = e.host.open("D1", &edge()).await.unwrap();
    e.host.close("D1").await;
    assert!(e.edges.gone_within("D1", Duration::from_secs(10)).await);
    assert_eq!(e.edges.methods("D1").last().unwrap(), "Browser.close");
    assert!(opened.edge.is_closed());
    assert!(e.host.open("D1", &edge()).await.unwrap().started);
    // The person closes the window: the next open starts Edge again on the same profile.
    e.edges.quit("D1");
    assert!(e.edges.gone_within("D1", Duration::from_secs(10)).await);
    assert!(e.host.open("D1", &edge()).await.unwrap().started);
    assert_eq!(e.edges.starts("D1").len(), 3);
    // Closing an Edge that does not run does nothing.
    e.host.close("D9").await;
}

#[tokio::test]
async fn its_pages_are_the_page_targets_of_its_list() {
    let targets: Vec<Value> = serde_json::from_str(JSON_LIST).unwrap();
    assert_eq!(
        page_urls(&targets),
        [
            "edge://sync-confirmation-dialog/",
            "http://127.0.0.1:54428/cart"
        ]
    );
    let e = env();
    let opened = e.host.open("D1", &edge()).await.unwrap();
    e.edges
        .set_pages("D1", &["https://shop.com/other", "https://shop.com/cart"]);
    assert_eq!(
        opened.edge.pages().await.unwrap(),
        ["https://shop.com/other", "https://shop.com/cart"]
    );
}

#[tokio::test]
async fn a_profile_held_by_another_edge_is_refused() {
    let e = env();
    // An Edge started outside Dots on the dot's profile: the fake, by hand.
    let mut outside = tokio::process::Command::new(exe());
    outside
        .arg(format!(
            "--user-data-dir={}",
            e.host.profile("D1").display()
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    dots_core::proc::hide_window(&mut outside);
    let _outside = outside.spawn().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !e.edges.running("D1") {
        assert!(
            std::time::Instant::now() < deadline,
            "the outside fake did not start"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        e.host.open("D1", &edge()).await.err(),
        Some(HostError::ProfileInUse)
    );
    assert_eq!(
        HostError::ProfileInUse.to_string(),
        "This dot's browser profile is in use by another Edge window; close it and try again."
    );
}

#[tokio::test]
async fn an_edge_or_a_node_that_is_not_there_is_said() {
    let e = env();
    let missing = e.dir.path().join("no-edge.exe");
    assert_eq!(
        e.host.open("D1", &missing).await.err(),
        Some(HostError::NotFound)
    );
    assert_eq!(
        HostError::NotFound.to_string(),
        "Microsoft Edge was not found; browser use needs it (Settings → Edge path)."
    );
    let nodeless = EdgeHost::with_launcher(&e.root, Launcher::Node("dots-no-such-node".into()));
    assert_eq!(
        nodeless.open("D1", &edge()).await.err(),
        Some(HostError::NoNode)
    );
    assert_eq!(HostError::NoNode.to_string(), NO_NODE);
    assert!(!e.edges.running("D1"));
    let installed = |key: &str| match key {
        "ProgramFiles(x86)" => Some("C:\\PF86".to_string()),
        "ProgramFiles" => Some("C:\\PF".to_string()),
        _ => None,
    };
    let under = |base: &str| {
        Path::new(base)
            .join("Microsoft")
            .join("Edge")
            .join("Application")
            .join("msedge.exe")
    };
    assert_eq!(
        edge_candidates(None, &installed),
        [under("C:\\PF86"), under("C:\\PF")]
    );
    assert_eq!(edge_candidates(Some("  "), &installed).len(), 2);
    assert_eq!(
        edge_candidates(Some(" D:/Edge/msedge.exe "), &installed),
        [PathBuf::from("D:/Edge/msedge.exe")]
    );
    assert_eq!(find_edge(Some(exe().as_str()), &installed), Some(edge()));
    // A path that is set but missing is not found: no fall back to an installed Edge.
    assert_eq!(find_edge(Some(missing.to_str().unwrap()), &installed), None);
}

/// Needs Node.js (`node`) on PATH, as browser use does.
#[tokio::test]
async fn the_helper_gives_edge_its_pipe_and_closes_it_when_dots_goes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("browser");
    let edges = FakeEdges::new(&root);
    let helped = EdgeHost::new(&root);
    let opened = helped.open("D1", &edge()).await.unwrap();
    assert!(opened.started);
    assert_eq!(
        std::fs::read_to_string(root.join(HELPER_FILE)).unwrap(),
        HELPER
    );
    let args: Vec<String> = serde_json::from_value(edges.starts("D1")[0]["args"].clone()).unwrap();
    assert_eq!(args[..2], PIPE_FLAGS);
    assert_eq!(args[2..], edge_args(&helped.profile("D1")));
    // Through the helper, on Edge's descriptors 3 and 4.
    edges.set_pages("D1", &["https://shop.com/cart"]);
    assert_eq!(
        opened.edge.pages().await.unwrap(),
        ["https://shop.com/cart"]
    );
    // Dots goes: the helper's stdin ends, and it closes Edge.
    drop((helped, opened));
    assert!(edges.gone_within("D1", Duration::from_secs(10)).await);
    let last = edges.received("D1").pop().unwrap();
    assert_eq!(last["method"], "Browser.close");
    assert_eq!(last["id"], 2147483647);
}

#[test]
fn finished_downloads_move_into_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("downloads");
    let to = dir.path().join("ws").join("downloads");
    std::fs::create_dir_all(&from).unwrap();
    std::fs::write(from.join("statement.pdf"), "new").unwrap();
    std::fs::write(from.join("report.csv"), "b").unwrap();
    std::fs::write(from.join("big.zip.crdownload"), "c").unwrap();
    std::fs::create_dir_all(&to).unwrap();
    std::fs::write(to.join("statement.pdf"), "old").unwrap();

    assert_eq!(
        move_downloads(&from, &to),
        ["report.csv", "statement (2).pdf"]
    );
    assert_eq!(
        std::fs::read_to_string(to.join("statement (2).pdf")).unwrap(),
        "new"
    );
    assert_eq!(
        std::fs::read_to_string(to.join("statement.pdf")).unwrap(),
        "old"
    );
    assert!(
        from.join("big.zip.crdownload").exists(),
        "still being written"
    );
    assert!(move_downloads(&from, &to).is_empty());
    assert!(move_downloads(&dir.path().join("none"), &to).is_empty());
}
```

**In `crates/dots-core/tests/browser_call_test.rs`, replace:**

```rust
use common::fake_browser::{cart_script, exe, playwright_command, FakeEdges, CART};
use common::fake_mcp::FakeMcp;
use common::{spec, temp_store};
use dots_core::approvals::{parked_message, ApprovalHub};
use dots_core::browser::gate::{classify, Ask, Danger, Kind, ASK_KEY, PASSWORD_REFUSAL};
use dots_core::browser::host::EdgeHost;
use dots_core::browser::{
    function_schemas, tools, Browsers, CallLimit, PlaywrightCommand, RunBrowser, NO_NODE,
    PAGE_MOVED, PLAYWRIGHT_MCP, TOO_LATE,
};
```

**with:**

```rust
use common::fake_browser::{cart_script, exe, host, playwright_command, FakeEdges, CART};
use common::fake_mcp::FakeMcp;
use common::{spec, temp_store};
use dots_core::approvals::{parked_message, ApprovalHub};
use dots_core::browser::gate::{classify, Ask, Danger, Kind, ASK_KEY, PASSWORD_REFUSAL};
use dots_core::browser::{
    function_schemas, tools, Browsers, CallLimit, PlaywrightCommand, RunBrowser, CDP_ENDPOINT_VAR,
    NO_NODE, PAGE_MOVED, PLAYWRIGHT_MCP, TOO_LATE,
};
```

**In `crates/dots-core/tests/browser_call_test.rs`, replace:**

```rust
/// Dropped in this order: the fakes quit, then the rest, then the folder.
struct Env {
```

**with:**

```rust
/// The port the runs' Playwright servers are told Dots' server is on. No server listens in
/// these tests: `fake-mcp` never connects.
const DOTS_PORT: u16 = 47_210;

/// Dropped in this order: the fakes quit, then the rest, then the folder.
struct Env {
```

**In `crates/dots-core/tests/browser_call_test.rs`, replace:**

```rust
    let browsers = Browsers::new(
        store.clone(),
        Arc::new(EdgeHost::new(&root)),
        playwright_command(&playwright),
    );
```

**with:**

```rust
    let browsers = Browsers::new(
        store.clone(),
        Arc::new(host(&root)),
        playwright_command(&playwright),
        DOTS_PORT,
    );
```

**In `crates/dots-core/tests/browser_call_test.rs`, replace:**

```rust
    assert_eq!(e.edges.starts(&dot.id).len(), 1);
    let port = e.edges.port(&dot.id).unwrap();
    let start = e
        .playwright
        .log()
        .into_iter()
        .find_map(|r| r.get("start").cloned())
        .unwrap();
    let args: Vec<String> = serde_json::from_value(start["args"].clone()).unwrap();
    assert_eq!(
        args[1..],
        [
            s("--cdp-endpoint"),
            format!("http://127.0.0.1:{port}"),
            s("--output-dir"),
            e.ws.join("downloads").to_string_lossy().into_owned(),
            s("--snapshot-mode"),
            s("none"),
        ]
    );
```

**with:**

```rust
    assert_eq!(e.edges.starts(&dot.id).len(), 1);
    let start = e
        .playwright
        .log()
        .into_iter()
        .find_map(|r| r.get("start").cloned())
        .unwrap();
    let args: Vec<String> = serde_json::from_value(start["args"].clone()).unwrap();
    assert_eq!(
        args[1..],
        [
            s("--output-dir"),
            e.ws.join("downloads").to_string_lossy().into_owned(),
            s("--snapshot-mode"),
            s("none"),
        ]
    );
    // Its Edge is its door on Dots' own server, with a secret of 128 bits, named in a
    // variable: no command line shows the secret.
    let endpoint = start["env"][CDP_ENDPOINT_VAR].as_str().unwrap();
    let secret = endpoint
        .strip_prefix(&format!("ws://127.0.0.1:{DOTS_PORT}/cdp/"))
        .unwrap_or_else(|| panic!("{start}"));
    assert_eq!(secret.len(), 22, "{start}");
    assert!(args.iter().all(|a| !a.contains(secret)), "{args:?}");
```

**In `crates/dots-core/tests/browser_call_test.rs`, replace:**

```rust
    let browsers = Browsers::new(
        e.store.clone(),
        Arc::new(EdgeHost::new(e.dir.path().join("browser"))),
        PlaywrightCommand {
            program: env!("CARGO_BIN_EXE_fake-mcp").to_string(),
            args: vec![script.to_string_lossy().into_owned()],
        },
    );
```

**with:**

```rust
    let browsers = Browsers::new(
        e.store.clone(),
        Arc::new(host(e.dir.path().join("browser"))),
        PlaywrightCommand {
            program: env!("CARGO_BIN_EXE_fake-mcp").to_string(),
            args: vec![script.to_string_lossy().into_owned()],
        },
        DOTS_PORT,
    );
```

**In `crates/dots-core/tests/browser_call_test.rs`, replace:**

```rust
    let no_node = Browsers::new(
        e.store.clone(),
        Arc::new(EdgeHost::new(e.dir.path().join("browser"))),
        PlaywrightCommand {
            program: s("dots-no-such-npx"),
            args: vec![],
        },
    );
```

**with:**

```rust
    let no_node = Browsers::new(
        e.store.clone(),
        Arc::new(host(e.dir.path().join("browser"))),
        PlaywrightCommand {
            program: s("dots-no-such-npx"),
            args: vec![],
        },
        DOTS_PORT,
    );
```

**In `crates/dots-core/src/bin/fake-mcp.rs`, replace:**

```rust
//! - `log`: a file that gets one JSON line per record: `{"start": {pid, args, cwd, env}}` (env:
//!   the variables whose names start with `FAKE_MCP_`), `{"msg": <message received>}` (over
//!   http with `"http": "POST"` and the `session`, `protocol` and `auth` headers), and
//!   `{"http": "GET" | "DELETE", "session": ...}`.
```

**with:**

```rust
//! - `log`: a file that gets one JSON line per record: `{"start": {pid, args, cwd, env}}` (env:
//!   the variables whose names start with `FAKE_MCP_` or `PLAYWRIGHT_MCP_`), `{"msg": <message
//!   received>}` (over http with `"http": "POST"` and the `session`, `protocol` and `auth`
//!   headers), and `{"http": "GET" | "DELETE", "session": ...}`.
```

**In `crates/dots-core/src/bin/fake-mcp.rs`, replace:**

```rust
        .filter(|(k, _)| k.starts_with("FAKE_MCP_"))
```

**with:**

```rust
        .filter(|(k, _)| k.starts_with("FAKE_MCP_") || k.starts_with("PLAYWRIGHT_MCP_"))
```

**In `crates/dots-core/tests/local_browser_test.rs`, replace:**

```rust
use common::fake_browser::{cart_script, exe, playwright_command, FakeEdges, CART};
```

**with:**

```rust
use common::fake_browser::{cart_script, exe, host, playwright_command, FakeEdges, CART};
```

**In `crates/dots-core/tests/local_browser_test.rs`, replace:**

```rust
use dots_core::browser::gate::PASSWORD_REFUSAL;
use dots_core::browser::host::EdgeHost;
```

**with:**

```rust
use dots_core::browser::gate::PASSWORD_REFUSAL;
```

**In `crates/dots-core/tests/local_browser_test.rs`, replace:**

```rust
    runner.set_browsers(Browsers::new(
        store.clone(),
        Arc::new(EdgeHost::new(&root)),
        playwright_command(&playwright),
    ));
```

**with:**

```rust
    // No server listens on port 1: `fake-mcp` never connects to its door.
    runner.set_browsers(Browsers::new(
        store.clone(),
        Arc::new(host(&root)),
        playwright_command(&playwright),
        1,
    ));
```

**In `crates/dots-core/tests/claude_browser_test.rs`, replace:**

```rust
use common::fake_browser::{cart_script, exe, playwright_command, FakeEdges, CART};
```

**with:**

```rust
use common::fake_browser::{cart_script, exe, host, playwright_command, FakeEdges, CART};
```

**In `crates/dots-core/tests/claude_browser_test.rs`, replace:**

```rust
use dots_core::approvals::{grant_key, ApprovalHub, DecideEffect};
use dots_core::browser::host::EdgeHost;
```

**with:**

```rust
use dots_core::approvals::{grant_key, ApprovalHub, DecideEffect};
```

**In `crates/dots-core/tests/claude_browser_test.rs`, replace:**

```rust
    let listener = bind(0).await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
```

**with:**

```rust
    let listener = bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
```

**In `crates/dots-core/tests/claude_browser_test.rs`, replace:**

```rust
    runner.set_browsers(Browsers::new(
        store.clone(),
        Arc::new(EdgeHost::new(&root)),
        playwright_command(&playwright),
    ));
```

**with:**

```rust
    runner.set_browsers(Browsers::new(
        store.clone(),
        Arc::new(host(&root)),
        playwright_command(&playwright),
        port,
    ));
```

**Append to `crates/dots-core/tests/claude_browser_test.rs`:**

```rust

/// Browser spec section 14: Dots' server has the doors' route, and a secret that opens no door
/// is refused with 404, WebSocket or not.
#[tokio::test]
async fn the_server_refuses_a_door_without_its_secret() {
    let e = env(json!([init(), success()])).await;
    let ws = e.base.replace("http://", "ws://");
    match tokio_tungstenite::connect_async(format!("{ws}/cdp/not-a-secret")).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(answer)) => {
            assert_eq!(answer.status().as_u16(), 404)
        }
        other => panic!("{:?}", other.map(|_| ())),
    }
    let plain = reqwest::get(format!("{}/cdp/not-a-secret", e.base))
        .await
        .unwrap();
    assert_eq!(plain.status().as_u16(), 404);
}
```

**In `crates/dots-core/tests/runtime_browser_test.rs`, replace:**

```rust
//! The runtime's browser commands (browser spec sections 6 to 8), with `fake-browser` as Edge:
//! "Open the browser" opens the dot's Edge on its profile and a second click finds it; the
//! status names Edge; a deleted dot's Edge closes and its profile goes; a run's screenshot is
//! read back as a `data:` URL, only under the name Playwright gives one.
```

**with:**

```rust
//! The runtime's browser commands (browser spec sections 6 to 8), with `fake-browser` as Edge,
//! started through the helper and Node.js (`node` on PATH) as Dots starts Edge: "Open the
//! browser" opens the dot's Edge on its pipe and profile, and a second click finds it; the
//! status names Edge; a deleted dot's Edge closes and its profile goes; a run's screenshot is
//! read back as a `data:` URL, only under the name Playwright gives one.
```

**In `crates/dots-core/tests/runtime_browser_test.rs`, replace:**

```rust
    let args: Vec<String> =
        serde_json::from_value(edges.starts(&dot.id)[0]["args"].clone()).unwrap();
    assert_eq!(
        args[0],
        format!("--user-data-dir={}", edges.profile(&dot.id).display())
    );
```

**with:**

```rust
    let args: Vec<String> =
        serde_json::from_value(edges.starts(&dot.id)[0]["args"].clone()).unwrap();
    assert_eq!(args[..2], dots_core::browser::host::PIPE_FLAGS);
    assert_eq!(
        args[2],
        format!("--user-data-dir={}", edges.profile(&dot.id).display())
    );
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test browser_host_test`
Expected: FAIL to compile: unresolved imports `dots_core::browser::host::Launcher`, `HELPER`, `HELPER_FILE`, `PIPE_FLAGS` (and `CDP_ENDPOINT_VAR` in `browser_call_test`).

- [ ] **Step 3: Start Edge on its pipe, and open a door for each Playwright server**

**Create `crates/dots-core/src/browser/browser-pipe.mjs`:**

```js
// Dots' private connection to a dot's Microsoft Edge (browser spec section 14), written by Dots
// into <data_dir>/browser/ and run as:
//
//   node browser-pipe.mjs <msedge.exe> <edge switches...>
//
// It starts Edge with --remote-debugging-pipe, so that Edge opens no port, and relays Edge's
// pipe (its file descriptors 3 and 4) to this program's stdin and stdout, byte for byte:
// messages are JSON, each ended by a NUL byte. When stdin ends (Dots stopped), Edge is asked
// to close, and ended after 5 s. When Edge ends, so does this program.
import { spawn } from 'node:child_process';

const [exe, ...args] = process.argv.slice(2);
// Without --edge-skip-compat-layer-relaunch, msedge.exe hands over to another process and
// the pipe is lost.
const edge = spawn(exe, ['--remote-debugging-pipe', '--edge-skip-compat-layer-relaunch', ...args], {
  stdio: ['ignore', 'ignore', 'ignore', 'pipe', 'pipe'],
});
const toEdge = edge.stdio[3];
const fromEdge = edge.stdio[4];
let midMessage = false;
let closing = false;

edge.on('error', (e) => {
  process.stderr.write(`browser-pipe: ${e.message}\n`);
  process.exit(1);
});
edge.on('exit', (code) => {
  process.exitCode = code ?? 0;
  process.stdin.destroy();
  toEdge.destroy();
  fromEdge.destroy();
});
toEdge.on('error', () => {});
fromEdge.on('error', () => {});

process.stdin.on('data', (chunk) => {
  if (closing || chunk.length === 0) return;
  midMessage = chunk[chunk.length - 1] !== 0;
  if (!toEdge.write(chunk)) {
    process.stdin.pause();
    toEdge.once('drain', () => process.stdin.resume());
  }
});
fromEdge.on('data', (chunk) => {
  if (!process.stdout.write(chunk)) {
    fromEdge.pause();
    process.stdout.once('drain', () => fromEdge.resume());
  }
});

function close() {
  if (closing) return;
  closing = true;
  // A message cut short is ended first, so that Edge reads the next one.
  if (midMessage) toEdge.write('\0');
  toEdge.write(JSON.stringify({ id: 2147483647, method: 'Browser.close' }) + '\0');
  setTimeout(() => edge.kill(), 5000).unref();
}
process.stdin.on('end', close);
process.stdin.on('error', close);
process.stdout.on('error', close);
```

**Create `crates/dots-core/src/browser/host.rs`:**

```rust
//! Microsoft Edge for dots (browser spec sections 2.1 and 14): one visible window per dot, on
//! the dot's own profile (`<root>/<dot_id>/profile`), with no debugging port. Dots starts Edge
//! through the helper `browser-pipe.mjs` (written into `<root>`), which gives Edge its
//! debugging pipe and relays it to the helper's stdin and stdout: Dots holds the only
//! connection (`pipe.rs`). Dots reads Edge's pages over it and closes Edge with
//! `Browser.close`; Playwright reaches Edge through its door (`door.rs`). An Edge lives at
//! most as long as the Dots that started it: when Dots stops, the helper's stdin ends and it
//! closes Edge.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::AsyncBufReadExt;

use super::pipe::Pipe;

/// The switches every dot's Edge starts with after its profile: no first-run pages, no sign-in
/// or sync of the person's account into the dot's profile, and pages that keep running while
/// the window is behind others (measured: without them every click timed out).
pub const EDGE_FLAGS: [&str; 7] = [
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-sync",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
    "--disable-features=CalculateNativeWinOcclusion",
];
/// The switches the helper puts first: the debugging pipe, and no hand-over to another
/// process, which would lose the pipe.
pub const PIPE_FLAGS: [&str; 2] = [
    "--remote-debugging-pipe",
    "--edge-skip-compat-layer-relaunch",
];
/// The helper, as Dots writes it into `<root>` before each start.
pub const HELPER: &str = include_str!("browser-pipe.mjs");
/// Its file name.
pub const HELPER_FILE: &str = "browser-pipe.mjs";
/// How long a started Edge has to answer on its pipe.
pub const START_WAIT: Duration = Duration::from_secs(20);
/// How long a closed Edge has to go.
pub const CLOSE_WAIT: Duration = Duration::from_secs(5);

/// Why a dot's Edge cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    #[error("Microsoft Edge was not found; browser use needs it (Settings → Edge path).")]
    NotFound,
    #[error(
        "This dot's browser profile is in use by another Edge window; close it and try again."
    )]
    ProfileInUse,
    #[error("Browser use needs Node.js (npx).")]
    NoNode,
    #[error("Microsoft Edge could not start: {0}")]
    Start(String),
    #[error("Microsoft Edge did not answer: {0}")]
    Unreachable(String),
}

/// A dot's Edge as `BrowserHost::open` found or started it.
#[derive(Debug, Clone)]
pub struct Opened {
    /// The connection to it.
    pub edge: Arc<Pipe>,
    /// It did not run: it was started now.
    pub started: bool,
}

/// The dots' Edges. Tests use `EdgeHost` with `fake-browser` as the program.
#[async_trait]
pub trait BrowserHost: Send + Sync {
    /// The dot's Edge: the one running on its profile, or `exe` started on it.
    async fn open(&self, dot_id: &str, exe: &Path) -> Result<Opened, HostError>;
    /// Closes the dot's Edge if it runs; returns once it is gone, or after `CLOSE_WAIT`.
    async fn close(&self, dot_id: &str);
    /// Where the dot's Edge saves downloads.
    fn downloads(&self, dot_id: &str) -> PathBuf;
    /// The folder Playwright servers start in: Dots' own, never a workspace, where a dot could
    /// plant a `node_modules` or an `.npmrc` for npx to follow.
    fn server_dir(&self) -> PathBuf;
    /// Closes the dot's Edge and removes its folder: profile and downloads (its dot was
    /// deleted).
    async fn remove(&self, dot_id: &str);
}

/// How a dot's Edge is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launcher {
    /// `<node> <root>/browser-pipe.mjs <msedge.exe> <switches>`: the helper gives Edge its
    /// pipe. `node` is found on `PATH` when it is a bare name.
    Node(String),
    /// The program speaks the pipe itself on its stdin and stdout (`fake-browser` in tests).
    Direct,
}

/// A dot's running Edge: its pipe, and the helper's process.
struct Running {
    pipe: Arc<Pipe>,
    child: tokio::process::Child,
}

/// Microsoft Edge, one per dot, each with a folder under `root` (`<data_dir>/browser`).
pub struct EdgeHost {
    root: PathBuf,
    launcher: Launcher,
    /// The running Edges. Held through a start or a close: two runs of a dot must not start
    /// two Edges on its profile.
    edges: tokio::sync::Mutex<HashMap<String, Running>>,
}

impl EdgeHost {
    /// Edges started through the helper, with `node`.
    pub fn new(root: impl Into<PathBuf>) -> EdgeHost {
        EdgeHost::with_launcher(root, Launcher::Node("node".to_string()))
    }

    pub fn with_launcher(root: impl Into<PathBuf>, launcher: Launcher) -> EdgeHost {
        EdgeHost {
            root: root.into(),
            launcher,
            edges: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// The dot's folder: its profile and downloads.
    pub fn dot_dir(&self, dot_id: &str) -> PathBuf {
        self.root.join(dot_id)
    }

    pub fn profile(&self, dot_id: &str) -> PathBuf {
        self.dot_dir(dot_id).join("profile")
    }

    /// The command that starts `exe` with `args` on a pipe.
    fn command(&self, exe: &Path, args: &[String]) -> Result<tokio::process::Command, HostError> {
        match &self.launcher {
            Launcher::Direct => {
                let mut command = tokio::process::Command::new(exe);
                command.args(args);
                Ok(command)
            }
            Launcher::Node(node) => {
                let node = find_program(node).ok_or(HostError::NoNode)?;
                let helper = self.root.join(HELPER_FILE);
                std::fs::create_dir_all(&self.root)
                    .and_then(|()| std::fs::write(&helper, HELPER))
                    .map_err(|e| HostError::Start(format!("{}: {e}", helper.display())))?;
                let mut command = tokio::process::Command::new(node);
                command.arg(helper).arg(exe).args(args);
                Ok(command)
            }
        }
    }
}

#[async_trait]
impl BrowserHost for EdgeHost {
    fn server_dir(&self) -> PathBuf {
        self.root.join("playwright")
    }

    async fn open(&self, dot_id: &str, exe: &Path) -> Result<Opened, HostError> {
        let mut edges = self.edges.lock().await;
        if let Some(running) = edges.get(dot_id).filter(|r| !r.pipe.is_closed()) {
            // Only while it answers: the person may have closed the window a moment ago, before
            // its pipe's end was read.
            if running
                .pipe
                .ask("Browser.getVersion", json!({}))
                .await
                .is_ok()
            {
                return Ok(Opened {
                    edge: running.pipe.clone(),
                    started: false,
                });
            }
        }
        // The person closed the window, or the helper ended.
        if let Some(gone) = edges.remove(dot_id) {
            reap(gone.child).await;
        }
        if !exe.is_file() {
            return Err(HostError::NotFound);
        }
        let profile = self.profile(dot_id);
        if profile_in_use(&profile) {
            return Err(HostError::ProfileInUse);
        }
        prepare_profile(&profile, &self.downloads(dot_id))
            .map_err(|e| HostError::Start(format!("its profile could not be prepared: {e}")))?;
        let mut command = self.command(exe, &edge_args(&profile))?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::proc::hide_window(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| HostError::Start(e.to_string()))?;
        let (Some(stdout), Some(stdin)) = (child.stdout.take(), child.stdin.take()) else {
            return Err(HostError::Start("its pipe could not be opened".to_string()));
        };
        log_errors(child.stderr.take(), dot_id);
        let pipe = Pipe::new(stdout, stdin);
        if let Err(e) = pipe
            .ask_within("Browser.getVersion", json!({}), START_WAIT)
            .await
        {
            let _ = child.start_kill();
            reap(child).await;
            // Edge hands over to the Edge that holds the profile, and closes its pipe.
            if profile_in_use(&profile) {
                return Err(HostError::ProfileInUse);
            }
            return Err(HostError::Start(format!(
                "it did not answer on its pipe: {e}"
            )));
        }
        edges.insert(
            dot_id.to_string(),
            Running {
                pipe: pipe.clone(),
                child,
            },
        );
        Ok(Opened {
            edge: pipe,
            started: true,
        })
    }

    async fn close(&self, dot_id: &str) {
        let mut edges = self.edges.lock().await;
        let Some(running) = edges.remove(dot_id) else {
            return;
        };
        if !running.pipe.is_closed() {
            let _ = running.pipe.ask("Browser.close", json!({})).await;
        }
        reap(running.child).await;
    }

    fn downloads(&self, dot_id: &str) -> PathBuf {
        self.dot_dir(dot_id).join("downloads")
    }

    async fn remove(&self, dot_id: &str) {
        self.close(dot_id).await;
        let dir = self.dot_dir(dot_id);
        // Edge lets go of its profile a moment after it ends.
        for _ in 0..20 {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => return,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }
        tracing::warn!("could not remove {}", dir.display());
    }
}

/// Waits up to `CLOSE_WAIT` for the helper (and its Edge) to end, then ends it.
async fn reap(mut child: tokio::process::Child) {
    if tokio::time::timeout(CLOSE_WAIT, child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
}

/// The helper's own errors go to the app log.
fn log_errors(stderr: Option<tokio::process::ChildStderr>, dot_id: &str) {
    let Some(stderr) = stderr else {
        return;
    };
    let dot_id = dot_id.to_string();
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::warn!(dot = %dot_id, "{line}");
        }
    });
}

/// The arguments of a dot's Edge: its profile, `EDGE_FLAGS`, and a blank page. The helper puts
/// `PIPE_FLAGS` before them.
pub fn edge_args(profile: &Path) -> Vec<String> {
    let mut args = vec![format!("--user-data-dir={}", profile.display())];
    args.extend(EDGE_FLAGS.iter().map(|flag| flag.to_string()));
    args.push("about:blank".to_string());
    args
}

fn is_page(target: &Value) -> bool {
    target.get("type").and_then(Value::as_str) == Some("page")
}

/// The address of every `page` target of a list of targets (`Target.getTargets`' `targetInfos`,
/// or a `/json/list` answer), in its order.
pub fn page_urls(targets: &[Value]) -> Vec<String> {
    targets
        .iter()
        .filter(|t| is_page(t))
        .filter_map(|t| t.get("url").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

/// Where Edge is looked for: `configured` (Settings' Edge path) when it is set, else
/// `%ProgramFiles(x86)%` and `%ProgramFiles%` under `Microsoft\Edge\Application\msedge.exe`.
pub fn edge_candidates(
    configured: Option<&str>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Vec<PathBuf> {
    if let Some(path) = configured.map(str::trim).filter(|p| !p.is_empty()) {
        return vec![PathBuf::from(path)];
    }
    ["ProgramFiles(x86)", "ProgramFiles"]
        .iter()
        .filter_map(|key| env(key))
        .map(|base| {
            Path::new(&base)
                .join("Microsoft")
                .join("Edge")
                .join("Application")
                .join("msedge.exe")
        })
        .collect()
}

/// The first of `edge_candidates` that is a file. A configured path that is not a file is not
/// found: Dots does not fall back to another Edge.
pub fn find_edge(
    configured: Option<&str>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    edge_candidates(configured, env)
        .into_iter()
        .find(|path| path.is_file())
}

/// A program as Dots starts it: a path that is a file, or a bare name found on `PATH` with
/// `PATHEXT` (`npx` is `npx.cmd`). `None` when there is none.
pub fn find_program(program: &str) -> Option<PathBuf> {
    let path = crate::mcp::resolve_program(
        program,
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("PATHEXT").as_deref(),
    );
    path.is_file().then_some(path)
}

/// Whether an Edge holds `profile`: Edge keeps `<profile>/lockfile` open so that no other
/// process can open it for writing.
pub fn profile_in_use(profile: &Path) -> bool {
    let lock = profile.join("lockfile");
    if !lock.exists() {
        return false;
    }
    match std::fs::OpenOptions::new().write(true).open(&lock) {
        Ok(file) => matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
        Err(_) => true,
    }
}

/// Makes the profile's folder and the download folder, and writes the download folder into the
/// profile's `Default/Preferences`, without a question per file. A `Preferences` that Dots
/// cannot read as a JSON object is Edge's and is left as it is.
fn prepare_profile(profile: &Path, downloads: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(profile.join("Default"))?;
    std::fs::create_dir_all(downloads)?;
    let prefs = profile.join("Default").join("Preferences");
    let mut value = match std::fs::read_to_string(&prefs) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(object @ Value::Object(_)) => object,
            _ => return Ok(()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e),
    };
    if !value["download"].is_object() {
        value["download"] = json!({});
    }
    value["download"]["default_directory"] = json!(downloads.to_string_lossy());
    value["download"]["prompt_for_download"] = json!(false);
    value["download"]["directory_upgrade"] = json!(true);
    std::fs::write(&prefs, value.to_string())
}

/// Moves every finished download in `from` into `to` (made when needed); a name taken there
/// gets ` (2)`, ` (3)`, … before its extension. A file Edge is still writing
/// (`.crdownload`, `.partial`, `.tmp`) stays. Returns the names the files have in `to`, by
/// name.
pub fn move_downloads(from: &Path, to: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(from) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !in_progress(name))
        .collect();
    names.sort();
    let mut moved = Vec::new();
    for name in names {
        if let Err(e) = std::fs::create_dir_all(to) {
            tracing::warn!("could not make {}: {e}", to.display());
            break;
        }
        let source = from.join(&name);
        let target = free_name(to, &name);
        let done = std::fs::rename(&source, &target).or_else(|_| {
            // Another drive: copy, then remove.
            std::fs::copy(&source, &target)?;
            std::fs::remove_file(&source)
        });
        match done {
            Ok(()) => moved.push(target.file_name().unwrap().to_string_lossy().into_owned()),
            Err(e) => tracing::warn!("could not move the download {name}: {e}"),
        }
    }
    moved
}

fn in_progress(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".crdownload")
        || name.ends_with(".partial")
        || name.ends_with(".tmp")
        || name.starts_with('.')
}

/// `dir/name`, or `dir/<stem> (<n>)<ext>` with the first `n` from 2 that is free.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|path| !path.exists())
        .expect("a free name")
}
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
use self::gate::{SiteRules, Verdict, ASK_KEY};
use self::host::{find_edge, find_program, move_downloads, BrowserHost, HostError};
use self::snapshot::PageMemory;
```

**with:**

```rust
use self::door::Doors;
use self::gate::{SiteRules, Verdict, ASK_KEY};
use self::host::{find_edge, find_program, move_downloads, BrowserHost, HostError};
use self::pipe::Pipe;
use self::snapshot::PageMemory;
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
impl PlaywrightCommand {
    /// Its arguments for a run: drive the Edge on `port`, write files into `output`, and put
    /// no page snapshot into an act's result (section 2.2; no `--caps`).
    pub fn args_for(&self, port: u16, output: &Path) -> Vec<String> {
        let mut args = self.args.clone();
        args.extend([
            "--cdp-endpoint".to_string(),
            format!("http://127.0.0.1:{port}"),
```

**with:**

```rust
/// The variable that tells a Playwright server where its Edge is: its door,
/// `ws://127.0.0.1:<dots port>/cdp/<secret>` (section 14). A variable rather than
/// `--cdp-endpoint`, so that the secret is on no command line that any program can list.
pub const CDP_ENDPOINT_VAR: &str = "PLAYWRIGHT_MCP_CDP_ENDPOINT";

impl PlaywrightCommand {
    /// Its arguments for a run: write files into `output`, and put no page snapshot into an
    /// act's result (section 2.2; no `--caps`). Its Edge is named by `CDP_ENDPOINT_VAR`.
    pub fn args_for(&self, output: &Path) -> Vec<String> {
        let mut args = self.args.clone();
        args.extend([
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
pub struct Browsers {
    store: Store,
    host: Arc<dyn BrowserHost>,
    playwright: PlaywrightCommand,
    users: tokio::sync::Mutex<HashMap<String, Users>>,
}

impl Browsers {
    pub fn new(
        store: Store,
        host: Arc<dyn BrowserHost>,
        playwright: PlaywrightCommand,
    ) -> Arc<Browsers> {
        Arc::new(Browsers {
            store,
            host,
            playwright,
            users: tokio::sync::Mutex::new(HashMap::new()),
        })
    }
```

**with:**

```rust
pub struct Browsers {
    store: Store,
    host: Arc<dyn BrowserHost>,
    playwright: PlaywrightCommand,
    /// The port of Dots' own server, where Playwright servers find their doors.
    port: u16,
    doors: Arc<Doors>,
    users: tokio::sync::Mutex<HashMap<String, Users>>,
}

impl Browsers {
    pub fn new(
        store: Store,
        host: Arc<dyn BrowserHost>,
        playwright: PlaywrightCommand,
        port: u16,
    ) -> Arc<Browsers> {
        Arc::new(Browsers {
            store,
            host,
            playwright,
            port,
            doors: Arc::new(Doors::default()),
            users: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// The doors of the runs' Playwright servers to their dots' Edges (section 14), which
    /// Dots' server serves at `/cdp/<secret>`.
    pub fn doors(&self) -> &Arc<Doors> {
        &self.doors
    }

    /// Where the Playwright server behind the door of `secret` connects.
    fn endpoint(&self, secret: &str) -> String {
        format!("ws://127.0.0.1:{}/cdp/{secret}", self.port)
    }
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
    async fn open(&self, dot_id: &str, hold: bool) -> Result<u16, HostError> {
```

**with:**

```rust
    async fn open(&self, dot_id: &str, hold: bool) -> Result<Arc<Pipe>, HostError> {
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
        if hold {
            user.runs += 1;
        }
        Ok(opened.port)
    }
```

**with:**

```rust
        if hold {
            user.runs += 1;
        }
        Ok(opened.edge)
    }
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
    closed: bool,
    /// The port of the Edge the session drives.
    port: Option<u16>,
    session: Option<Arc<McpSession>>,
}
```

**with:**

```rust
    closed: bool,
    /// The Edge the session drives.
    edge: Option<Arc<Pipe>>,
    /// The secret of the session's door to it.
    secret: Option<String>,
    session: Option<Arc<McpSession>>,
}
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
    /// The Playwright session of the next call: Edge found or started, and the server started at
    /// the first call, again after the person closed Edge (a new port), and again after it
    /// stopped.
    async fn ready(&self) -> Result<Arc<McpSession>, String> {
        let mut live = self.live.lock().await;
        if live.closed {
            return Err("The run has ended.".to_string());
        }
        let port = self
            .browsers
            .open(&self.dot_id, !live.held)
            .await
            .map_err(|e| e.to_string())?;
        live.held = true;
        if live.port != Some(port) {
            // Another Edge: what the run knew of its pages is gone.
            *self.memory.lock().unwrap() = PageMemory::default();
            if let Some(old) = live.session.take() {
                old.close().await;
            }
            live.port = Some(port);
        }
        if let Some(session) = live.session.clone().filter(|s| !s.is_stopped()) {
            return Ok(session);
        }
        if let Some(old) = live.session.take() {
            old.close().await;
        }
        let session = Arc::new(self.start_server(port).await?);
        live.session = Some(session.clone());
        Ok(session)
    }

    /// Starts the Playwright server on the Edge on `port`, in Dots' own folder for it.
    async fn start_server(&self, port: u16) -> Result<McpSession, String> {
```

**with:**

```rust
    /// The Playwright session of the next call: Edge found or started, and the server started at
    /// the first call, again after the person closed Edge (another Edge), and again after it
    /// stopped. Each server gets a door of its own to the Edge.
    async fn ready(&self) -> Result<Arc<McpSession>, String> {
        let mut live = self.live.lock().await;
        if live.closed {
            return Err("The run has ended.".to_string());
        }
        let edge = self
            .browsers
            .open(&self.dot_id, !live.held)
            .await
            .map_err(|e| e.to_string())?;
        live.held = true;
        if !live
            .edge
            .as_ref()
            .is_some_and(|known| Arc::ptr_eq(known, &edge))
        {
            // Another Edge: what the run knew of its pages is gone.
            *self.memory.lock().unwrap() = PageMemory::default();
            self.stop_server(&mut live).await;
            live.edge = Some(edge.clone());
        }
        if let Some(session) = live.session.clone().filter(|s| !s.is_stopped()) {
            return Ok(session);
        }
        self.stop_server(&mut live).await;
        let secret = self.browsers.doors.open(edge);
        live.secret = Some(secret.clone());
        let session = Arc::new(self.start_server(&secret).await?);
        live.session = Some(session.clone());
        Ok(session)
    }

    /// Stops the run's Playwright server, if one runs, and shuts its door.
    async fn stop_server(&self, live: &mut Live) {
        if let Some(old) = live.session.take() {
            old.close().await;
        }
        if let Some(secret) = live.secret.take() {
            self.browsers.doors.shut(&secret);
        }
    }

    /// Starts the Playwright server on the Edge behind the door of `secret`, in Dots' own
    /// folder for it.
    async fn start_server(&self, secret: &str) -> Result<McpSession, String> {
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
            args: command.args_for(port, &self.workspace.join("downloads")),
            cwd: Some(dir.to_string_lossy().into_owned()),
            env: BTreeMap::new(),
```

**with:**

```rust
            args: command.args_for(&self.workspace.join("downloads")),
            cwd: Some(dir.to_string_lossy().into_owned()),
            env: BTreeMap::from([(CDP_ENDPOINT_VAR.to_string(), self.browsers.endpoint(secret))]),
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
    async fn page_now(&self) -> Option<String> {
        let port = self.live.lock().await.port?;
        let pages = self.browsers.host.pages(port).await.ok()?;
```

**with:**

```rust
    async fn page_now(&self) -> Option<String> {
        let edge = self.live.lock().await.edge.clone()?;
        let pages = edge.pages().await.ok()?;
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
        live.closed = true;
        if let Some(session) = live.session.take() {
            session.close().await;
        }
        if live.held {
```

**with:**

```rust
        live.closed = true;
        self.stop_server(&mut live).await;
        if live.held {
```

**In `crates/dots-core/src/server/mod.rs`, replace:**

```rust
        .route(
            "/mcp/s/{server}",
            post(gateway::handle)
                .get(|| async { StatusCode::METHOD_NOT_ALLOWED })
                .layer(DefaultBodyLimit::max(MAX_MCP_BODY)),
        )
```

**with:**

```rust
        .route(
            "/mcp/s/{server}",
            post(gateway::handle)
                .get(|| async { StatusCode::METHOD_NOT_ALLOWED })
                .layer(DefaultBodyLimit::max(MAX_MCP_BODY)),
        )
        // The door of a run's Playwright to its dot's Edge (browser spec section 14).
        .route("/cdp/{secret}", get(browser::door))
```

**In `crates/dots-core/src/server/browser.rs`, replace:**

```rust
use std::time::Instant;

use axum::response::{IntoResponse, Response};
use axum::Json;
```

**with:**

```rust
use std::time::Instant;

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
```

**Append to `crates/dots-core/src/server/browser.rs`:**

```rust

/// `GET /cdp/<secret>`: the door of a run's Playwright server to its dot's Edge (browser spec
/// section 14). Without the dots' browsers, every secret is wrong.
pub(super) async fn door(
    State(s): State<ServerState>,
    Path(secret): Path<String>,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let Some(browsers) = s.runner.browsers() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    crate::browser::door::connect(browsers.doors().clone(), secret, upgrade).await
}
```

**In `crates/dots-core/src/runtime.rs`, replace:**

```rust
            crate::browser::PlaywrightCommand::default(),
        );
        runner.set_browsers(browsers.clone());
```

**with:**

```rust
            crate::browser::PlaywrightCommand::default(),
            port,
        );
        runner.set_browsers(browsers.clone());
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test browser_host_test --test browser_call_test --test claude_browser_test --test local_browser_test --test runtime_browser_test --test browser_door_test --test browser_pipe_test`
Expected: PASS (8, 13, 4, 4, 3, 4 and 5 tests). `tasklist | grep -c fake-browser` prints `0`.

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt`, `cargo fmt --check`.
Expected: all pass but the known failures; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/bin/fake-browser.rs crates/dots-core/src/bin/fake-mcp.rs crates/dots-core/src/browser/browser-pipe.mjs crates/dots-core/src/browser/host.rs crates/dots-core/src/browser/mod.rs crates/dots-core/src/server/mod.rs crates/dots-core/src/server/browser.rs crates/dots-core/src/runtime.rs crates/dots-core/tests/common/fake_browser.rs crates/dots-core/tests/browser_host_test.rs crates/dots-core/tests/browser_call_test.rs crates/dots-core/tests/local_browser_test.rs crates/dots-core/tests/claude_browser_test.rs crates/dots-core/tests/runtime_browser_test.rs
git commit -m "feat(core): Edge on a pipe through a Node helper, reached by Playwright through Dots' server"
```

### Task 4: One run at a time, and a spent door

A dot's browser is held by one run, from its first browser call until it ends: a browser call of another run of the dot fails with "This dot's browser is in use by another run; try again when it ends." and starts nothing; the holder's end closes Edge unless the person opened it. A run whose Playwright lost its connection cannot bring it back through its spent door: the run's next browser call stops that server and starts a new one with a new door.

**Files:**

- Modify: `crates/dots-core/tests/browser_call_test.rs` (an import; one test replaced)
- Modify: `crates/dots-core/tests/browser_door_test.rs` (imports; one test appended)
- Modify: `crates/dots-core/src/browser/host.rs` (`HostError::InUse`)
- Modify: `crates/dots-core/src/browser/mod.rs` (`Users`, `Browsers::{open, release}`, `RunBrowser::{ready, close}`)

**Interfaces:**

- Consumes: Task 3's `Browsers`, `RunBrowser`, `Doors::spent`.
- Produces: `HostError::InUse` (the sentence above); `Browsers::open(&self, dot_id: &str, run_id: &str)` and `Browsers::release(&self, dot_id: &str, run_id: &str)` (private).

- [ ] **Step 1: Write the failing tests**

**In `crates/dots-core/tests/browser_call_test.rs`, replace:**

```rust
use dots_core::browser::gate::{classify, Ask, Danger, Kind, ASK_KEY, PASSWORD_REFUSAL};
```

**with:**

```rust
use dots_core::browser::gate::{classify, Ask, Danger, Kind, ASK_KEY, PASSWORD_REFUSAL};
use dots_core::browser::host::HostError;
```

**In `crates/dots-core/tests/browser_call_test.rs`, replace:**

```rust
#[tokio::test]
async fn edge_is_held_while_a_run_uses_it_and_closes_after_the_last() {
    let e = env().await;
    let dot = dot(&e, &[], 0).await;
    // Two Playwright servers at once: a script without the `alive` lock, which a second
    // `fake-mcp` of the same script would wait for.
    let script = e.dir.path().join("two-runs.json");
    std::fs::write(&script, cart_script().to_string()).unwrap();
    let browsers = Browsers::new(
        e.store.clone(),
        Arc::new(host(e.dir.path().join("browser"))),
        PlaywrightCommand {
            program: env!("CARGO_BIN_EXE_fake-mcp").to_string(),
            args: vec![script.to_string_lossy().into_owned()],
        },
        DOTS_PORT,
    );
    let run_of = |run: &Run| browsers.for_run(&dot, &run.id, &e.ws);
    let (r1, _) = run(&e, &dot).await;
    let (r2, _) = run(&e, &dot).await;
    let (first, second) = (run_of(&r1), run_of(&r2));
    call(&e, &first, "browser_snapshot", json!({})).await;
    call(&e, &second, "browser_snapshot", json!({})).await;
    assert_eq!(
        e.edges.starts(&dot.id).len(),
        1,
        "the second run uses the same Edge"
    );
    first.close().await;
    assert!(e.edges.running(&dot.id), "the other run still holds it");
    second.close().await;
    assert!(e.edges.gone_within(&dot.id, Duration::from_secs(10)).await);
    // Opened by the person, it stays open after the runs that used it.
    browsers.open_for_person(&dot.id).await.unwrap();
    let (r3, _) = run(&e, &dot).await;
    let third = run_of(&r3);
    call(&e, &third, "browser_snapshot", json!({})).await;
    third.close().await;
    assert!(e.edges.running(&dot.id));
    assert_eq!(e.edges.starts(&dot.id).len(), 2);
    // A run that never called its browser holds nothing.
    let (r4, _) = run(&e, &dot).await;
    let idle = run_of(&r4);
    idle.close().await;
    assert!(e.edges.running(&dot.id));
    // After the end, a call does nothing.
    assert!(failed(
        &call(&e, &third, "browser_snapshot", json!({})).await
    ));
}
```

**with:**

```rust
#[tokio::test]
async fn a_dots_browser_serves_one_run_at_a_time() {
    let e = env().await;
    let dot = dot(&e, &[], 0).await;
    // Playwright servers of several runs: a script without the `alive` lock, which a second
    // `fake-mcp` of the same script would wait for.
    let script = e.dir.path().join("two-runs.json");
    std::fs::write(&script, cart_script().to_string()).unwrap();
    let browsers = Browsers::new(
        e.store.clone(),
        Arc::new(host(e.dir.path().join("browser"))),
        PlaywrightCommand {
            program: env!("CARGO_BIN_EXE_fake-mcp").to_string(),
            args: vec![script.to_string_lossy().into_owned()],
        },
        DOTS_PORT,
    );
    let run_of = |run: &Run| browsers.for_run(&dot, &run.id, &e.ws);
    let (r1, _) = run(&e, &dot).await;
    let (r2, _) = run(&e, &dot).await;
    let (first, second) = (run_of(&r1), run_of(&r2));
    assert!(!failed(
        &call(&e, &first, "browser_snapshot", json!({})).await
    ));
    // A second run of the dot is refused while the first holds its browser.
    let refused = call(&e, &second, "browser_snapshot", json!({})).await;
    assert!(failed(&refused));
    assert_eq!(
        text(&refused),
        "This dot's browser is in use by another run; try again when it ends."
    );
    assert_eq!(text(&refused), HostError::InUse.to_string());
    assert_eq!(e.edges.starts(&dot.id).len(), 1);
    // A refused run held nothing: its end leaves the first run's Edge open.
    second.close().await;
    assert!(e.edges.running(&dot.id), "the first run still holds it");
    first.close().await;
    assert!(e.edges.gone_within(&dot.id, Duration::from_secs(10)).await);
    // Once the holder ended, another run gets the browser.
    let (r3, _) = run(&e, &dot).await;
    let third = run_of(&r3);
    assert!(!failed(
        &call(&e, &third, "browser_snapshot", json!({})).await
    ));
    assert_eq!(e.edges.starts(&dot.id).len(), 2);
    // Opened by the person meanwhile, it stays open after the run that held it.
    browsers.open_for_person(&dot.id).await.unwrap();
    third.close().await;
    assert!(e.edges.running(&dot.id));
    let (r4, _) = run(&e, &dot).await;
    let fourth = run_of(&r4);
    assert!(!failed(
        &call(&e, &fourth, "browser_snapshot", json!({})).await
    ));
    fourth.close().await;
    assert!(e.edges.running(&dot.id));
    assert_eq!(e.edges.starts(&dot.id).len(), 2);
    // After the end, a call does nothing.
    assert!(failed(
        &call(&e, &third, "browser_snapshot", json!({})).await
    ));
}
```

**In `crates/dots-core/tests/browser_door_test.rs`, replace:**

```rust
use common::edge_pipe::TestEdge;
use dots_core::browser::door::{self, new_secret, Doors};
use dots_core::browser::pipe::MAX_MESSAGE;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
```

**with:**

```rust
use common::edge_pipe::TestEdge;
use common::fake_browser::{cart_script, exe, host, playwright_command, FakeEdges, CART};
use common::fake_mcp::FakeMcp;
use common::{spec, temp_store};
use dots_core::approvals::ApprovalHub;
use dots_core::browser::door::{self, new_secret, Doors};
use dots_core::browser::pipe::MAX_MESSAGE;
use dots_core::browser::{Browsers, CallLimit, CDP_ENDPOINT_VAR};
use dots_core::events::new_bus;
use dots_core::model::{NewRun, TriggerKind, WorkspaceMode};
use dots_core::settings::Settings;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_util::sync::CancellationToken;
```

**Append to `crates/dots-core/tests/browser_door_test.rs`:**

```rust

/// Playwright's connection dropped and it would come back with the same secret: the door is
/// spent, so the run's next browser call starts a new Playwright server with a new door.
#[tokio::test]
async fn a_run_whose_playwright_lost_its_door_gets_a_new_one() {
    let (dir, store) = temp_store().await;
    store
        .put_settings(&Settings {
            edge_path: Some(exe()),
            ..Settings::default()
        })
        .await
        .unwrap();
    let root = dir.path().join("browser");
    let edges = FakeEdges::new(&root);
    let playwright = FakeMcp::new(cart_script());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let browsers = Browsers::new(
        store.clone(),
        Arc::new(host(&root)),
        playwright_command(&playwright),
        port,
    );
    serve_on(listener, browsers.doors().clone());
    let mut s = spec("shopper");
    s.workdir = dir.path().to_string_lossy().into_owned();
    s.workspace_mode = WorkspaceMode::Folder;
    s.browser = true;
    let dot = store.create_dot(&s).await.unwrap();
    edges.first_pages(&dot.id, &[CART]);
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let browser = browsers.for_run(&dot, &run.id, dir.path());
    let hub = ApprovalHub::new(store.clone(), new_bus());
    let snapshot = || async {
        browser
            .call(
                &hub,
                "browser_snapshot",
                json!({}),
                CallLimit::PerformWithin(Duration::from_secs(30)),
                &CancellationToken::new(),
            )
            .await
    };
    let endpoints = || -> Vec<String> {
        playwright
            .log()
            .iter()
            .filter_map(|r| r.get("start"))
            .map(|start| start["env"][CDP_ENDPOINT_VAR].as_str().unwrap().to_string())
            .collect()
    };
    assert_ne!(snapshot().await["isError"], true);
    let first = endpoints().pop().unwrap();
    let secret = first.rsplit('/').next().unwrap().to_string();
    // Playwright comes in through its door, and its connection drops.
    let (mut socket, _) = tokio_tungstenite::connect_async(&first).await.unwrap();
    socket.close(None).await.unwrap();
    assert!(spent_within(browsers.doors(), &secret).await);
    assert_eq!(enter(port, &secret).await.err(), Some(404));
    // The next call starts a new server, with a door of its own; the old secret stays out.
    assert_ne!(snapshot().await["isError"], true);
    let now = endpoints();
    assert_eq!(now.len(), 2);
    assert_ne!(now[1], first);
    assert_eq!(enter(port, &secret).await.err(), Some(404));
    let mut again = enter(port, now[1].rsplit('/').next().unwrap())
        .await
        .unwrap();
    assert_eq!(edges.starts(&dot.id).len(), 1, "the same Edge");
    // The run ends: its Edge closes, and with it the connection.
    browser.close().await;
    assert!(edges.gone_within(&dot.id, Duration::from_secs(10)).await);
    let closed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match again.next().await {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(closed.is_ok());
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p dots-core --test browser_call_test`, then `cargo test -p dots-core --test browser_door_test`
Expected: the first FAILS to compile: no variant `InUse` in `HostError`. The second compiles and `a_run_whose_playwright_lost_its_door_gets_a_new_one` FAILS: `left: 1, right: 2` (the run kept the server whose door is spent).

- [ ] **Step 3: Hold the browser for one run, and replace a spent door**

**In `crates/dots-core/src/browser/host.rs`, replace:**

```rust
    #[error("Browser use needs Node.js (npx).")]
    NoNode,
```

**with:**

```rust
    #[error("Browser use needs Node.js (npx).")]
    NoNode,
    #[error("This dot's browser is in use by another run; try again when it ends.")]
    InUse,
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
/// Who uses a dot's Edge: its runs that called the browser, and the person.
#[derive(Debug, Default)]
struct Users {
    runs: usize,
    person: bool,
}

/// The dots' browsers (section 2.1): one Edge per dot, shared by the dot's runs, closed when the
/// last run that used it ends, unless the person opened it ("Open the browser").
```

**with:**

```rust
/// Who uses a dot's Edge: the run that holds it, and the person.
#[derive(Debug, Default)]
struct Users {
    /// The run that holds the browser, from its first browser call until it ends.
    run: Option<String>,
    person: bool,
}

/// The dots' browsers (sections 2.1 and 14): one Edge per dot, serving one run of the dot at a
/// time, closed when that run ends, unless the person opened it ("Open the browser").
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
    /// The dot's Edge for a run: found, or started. `hold` counts the run in (its first call);
    /// a later call only finds it again, or starts it after the person closed it.
    async fn open(&self, dot_id: &str, hold: bool) -> Result<Arc<Pipe>, HostError> {
        let exe = self.edge().await?;
        let mut users = self.users.lock().await;
        let opened = self.host.open(dot_id, &exe).await?;
        let user = users.entry(dot_id.to_string()).or_default();
        if opened.started {
            user.person = false;
        }
        if hold {
            user.runs += 1;
        }
        Ok(opened.edge)
    }

    /// A run is done with the dot's Edge: the last one closes it, unless the person opened it.
    async fn release(&self, dot_id: &str) {
        let mut users = self.users.lock().await;
        let Some(user) = users.get_mut(dot_id) else {
            return;
        };
        user.runs = user.runs.saturating_sub(1);
        if user.runs == 0 && !user.person {
            users.remove(dot_id);
            self.host.close(dot_id).await;
        }
    }
```

**with:**

```rust
    /// The dot's Edge for the run `run_id`: found, or started, again after the person closed
    /// it. The run holds the browser from then on; while another run holds it, `InUse`.
    async fn open(&self, dot_id: &str, run_id: &str) -> Result<Arc<Pipe>, HostError> {
        let exe = self.edge().await?;
        let mut users = self.users.lock().await;
        let user = users.entry(dot_id.to_string()).or_default();
        if user.run.as_deref().is_some_and(|holder| holder != run_id) {
            return Err(HostError::InUse);
        }
        let opened = self.host.open(dot_id, &exe).await?;
        let user = users.entry(dot_id.to_string()).or_default();
        if opened.started {
            user.person = false;
        }
        user.run = Some(run_id.to_string());
        Ok(opened.edge)
    }

    /// The run `run_id` is done with the dot's Edge: it closes, unless the person opened it.
    async fn release(&self, dot_id: &str, run_id: &str) {
        let mut users = self.users.lock().await;
        let Some(user) = users.get_mut(dot_id) else {
            return;
        };
        if user.run.as_deref() != Some(run_id) {
            return;
        }
        user.run = None;
        if !user.person {
            users.remove(dot_id);
            self.host.close(dot_id).await;
        }
    }
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
        let edge = self
            .browsers
            .open(&self.dot_id, !live.held)
            .await
            .map_err(|e| e.to_string())?;
```

**with:**

```rust
        let edge = self
            .browsers
            .open(&self.dot_id, &self.run_id)
            .await
            .map_err(|e| e.to_string())?;
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
        if let Some(session) = live.session.clone().filter(|s| !s.is_stopped()) {
            return Ok(session);
        }
        self.stop_server(&mut live).await;
        let secret = self.browsers.doors.open(edge);
```

**with:**

```rust
        // A Playwright whose connection ended cannot come in again: its secret is spent.
        let spent = live
            .secret
            .as_deref()
            .is_some_and(|secret| self.browsers.doors.spent(secret));
        if let Some(session) = live.session.clone().filter(|s| !s.is_stopped() && !spent) {
            return Ok(session);
        }
        self.stop_server(&mut live).await;
        let secret = self.browsers.doors.open(edge);
```

**In `crates/dots-core/src/browser/mod.rs`, replace:**

```rust
            live.held = false;
            self.browsers.release(&self.dot_id).await;
```

**with:**

```rust
            live.held = false;
            self.browsers.release(&self.dot_id, &self.run_id).await;
```

- [ ] **Step 4: Run the task's tests**

Run: `cargo test -p dots-core --test browser_call_test --test browser_door_test --test browser_host_test --test claude_browser_test --test local_browser_test --test runtime_browser_test`
Expected: PASS (13, 5, 8, 4, 4 and 3 tests).

- [ ] **Step 5: Run the checks**

Run: `cargo test -p dots-core --no-fail-fast`, `cargo test -p dots-app`, `cargo clippy --workspace --all-targets`, `cargo fmt`, `cargo fmt --check`.
Expected: all pass but the known failures; no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/browser/host.rs crates/dots-core/src/browser/mod.rs crates/dots-core/tests/browser_call_test.rs crates/dots-core/tests/browser_door_test.rs
git commit -m "feat(core): a dot's browser serves one run at a time, and a spent door gets a new one"
```

### Task 5: Bring the spec in line with what was built

Section 14 of the browser spec gains the changes this plan made (the table "Deviations from the spec", and any fix a task's step recorded in its commit body); section 11's open risk of the port becomes the risks that remain; section 13's notes on the port and `/json/list` point to section 14. The smoke is run separately, after this plan, as the browser plan's Task 13 is; this task only records what it adds.

**Files:**

- Modify: `docs/superpowers/specs/2026-10-21-browser-design.md` (section 11, two bullets of section 13, section 14 appended)

**Interfaces:**

- Consumes: the Deviations table of this plan; `git log --format=%B 5de4432..feat/browser` for fixes recorded by Tasks 1 to 4.
- Produces: a spec that matches the code.

- [ ] **Step 1: Edit the spec**

**In `docs/superpowers/specs/2026-10-21-browser-design.md`, replace:**

```markdown
- **The debugging port has no password.** Edge listens on `127.0.0.1:<port>` with no authentication, so any program on this computer, including a shell tool of any dot, can drive a dot's logged-in Edge and read its cookies, around the gate. Until Dots holds the connection itself (a pipe with an authenticating proxy), a dot whose policy lets it run commands should not be given the browser of a dot whose logins matter.
```

**with:**

```markdown
- **The door's secret.** Since section 14 Edge has no port: Dots holds its only connection, and a run's Playwright comes in through a door on Dots' server whose secret lets one connection in. The secret is in the Playwright server's environment, not on its command line, but a program running as the person can read another process's memory, take the secret and come in before Playwright does (Playwright's call then fails). On a computer where such programs run, a dot whose policy lets it run commands should still not be given the browser of a dot whose logins matter.
- **Dots' server down.** Playwright reaches Edge through Dots' local server: when the server could not start (its port taken), browser calls fail on both engines.
```

**In `docs/superpowers/specs/2026-10-21-browser-design.md`, replace:**

```markdown
and closes it with `PUT /json/close/<id>` for each page. A profile is in use when its `lockfile` cannot be opened for writing (section 2.1).
```

**with:**

```markdown
and closes it with `PUT /json/close/<id>` for each page. A profile is in use when its `lockfile` cannot be opened for writing (section 2.1). Section 14 replaced the port and its file: Dots holds Edge's pipe and closes Edge with `Browser.close`.
```

**In `docs/superpowers/specs/2026-10-21-browser-design.md`, replace:**

```markdown
Only Playwright's own result sections move the current page: console messages, found text and the results of code cannot (section 3.2).
```

**with:**

```markdown
Only Playwright's own result sections move the current page: console messages, found text and the results of code cannot (section 3.2). Since section 14 the addresses are Edge's `Target.getTargets` (type `page`), read over its pipe.
```

**Append to `docs/superpowers/specs/2026-10-21-browser-design.md`:**

```markdown

The plan (`docs/superpowers/plans/2026-10-21-browser-pipe.md`) built this section with these changes, after measuring Edge 155, Node.js 24 and `@playwright/mcp@0.0.83` (Playwright 1.64) on this PC:

- The Playwright server finds its door in the variable `PLAYWRIGHT_MCP_CDP_ENDPOINT`, which 0.0.83 reads as `--cdp-endpoint`, not on its command line: any program can list command lines (`Get-CimInstance Win32_Process`).
- When a Playwright goes, Dots detaches the sessions Edge attached for it and turns Edge's auto-attach off; a new Playwright on the same Edge lets the one before go first. Measured: otherwise the next Playwright found no page, since the pipe is one connection for the whole life of Edge.
- A secret is made for each start of a Playwright server. A request with the right secret that is not a WebSocket spends the door and gets the upgrade's own rejection. A Playwright whose connection ended cannot come in again: the run's next browser call starts a new server with a new door.
- Edge closes when the run that holds it ends, unless the person opened it, and when Dots stops: the helper's stdin ends, it sends `Browser.close` and ends Edge after 5 s. The helper puts `--remote-debugging-pipe --edge-skip-compat-layer-relaunch` before Dots' switches.
- A message may be up to 256 MB either way; a longer one from Edge is dropped and logged.
- Node.js missing fails the call with the sentence of `NO_NODE` before anything starts.
- `fake-browser` speaks the pipe on its stdin and stdout, or on its descriptors 3 and 4 under the real helper (the helper's test and the runtime's tests need `node`); its test controls are on an HTTP port of its own that Dots never uses. The pipe's own logic is tested with the test playing Edge in memory.
- The smoke adds: no `msedge.exe` whose command line names a dot's profile holds a listening port (`Get-NetTCPConnection -State Listen -OwningProcess <pid>` for each); no Playwright command line holds `/cdp/`; a browser call of a second run of the dot during a run fails with "This dot's browser is in use by another run; try again when it ends."; quitting Dots closes the dot's Edge.
```

Add one bullet per fix a task's commit body names (Tasks 1 to 4) to the appended list, in the form of the others.

- [ ] **Step 2: Run the checks**

Run: `git diff --check`, then `grep -c "PLAYWRIGHT_MCP_CDP_ENDPOINT" docs/superpowers/specs/2026-10-21-browser-design.md`
Expected: no whitespace errors; `1`.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-10-21-browser-design.md
git commit -m "docs(spec): the private connection to Edge as built"
```

**The smoke, run separately** (not a task of this plan; it extends the browser smoke `docs/verification/2026-10-21-browser-smoke.md` and is recorded in a new `docs/verification/<date>-browser-pipe-smoke.md`): with a real Edge, Node.js and `@playwright/mcp@0.0.83`, a throwaway data folder and never the person's profile, while a dot's browser is open: `Get-CimInstance Win32_Process -Filter "name='msedge.exe'"` lists its Edge with `--remote-debugging-pipe` and no `--remote-debugging-port`, and `Get-NetTCPConnection -State Listen -OwningProcess <pid>` is empty for every one of its processes; Playwright's `node` command line holds no `/cdp/`; a second run of the dot started during the first fails its browser call with "This dot's browser is in use by another run; try again when it ends."; closing the Edge window mid-run and calling again starts Edge again; quitting Dots closes the dot's Edge within about 5 s; the earlier smoke's steps 2 to 4 (a page read, a first-time site, "Place order" asking, a download) pass as before.
