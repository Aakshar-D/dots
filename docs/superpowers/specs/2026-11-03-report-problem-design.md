# Report a problem

- Date: 2026-11-03
- Status: approved (owner decided the design, 2026-11-03)
- Builds on: the app log (`apps/dots-app/src/startup.rs`: daily `dots.*.log` in `<data dir>/logs`, the last seven kept, every line through `vault::redact::RedactWriter`), the env vault's redactor (`vault/redact.rs`), memory's secret patterns (`memory::looks_secret`).

## 1. Goal

A person who hits a problem can hand over a short plain-text report that says what went wrong and what Dots was doing, without anything private in it. Dots never sends it anywhere: the person copies it or saves it to a file and sends it themselves.

What happens today: the only pointer to the log is the log folder line on the fatal-error page. Someone who wants to report a problem has to find the folder, pick a file, and read it for secrets themselves.

Success: from Settings, one dialog shows exactly the text that will be shared, and the person can copy it or save it.

## 2. Behaviour (decided with the owner)

1. **Entry:** Settings gets a **Help** section (after Updates) with a **Report a problem** button. Settings is where the version and Updates live; the app has no About page or main menu.
2. **The dialog:** a text box **"What went wrong?"**, a checkbox **"Include recent log lines"** (on by default), and a read-only preview of the exact report. The preview is rebuilt when the box or the checkbox changes (debounced).
3. **Actions:** **Copy** (to the clipboard) and **Save to file…** (the native save dialog, default name `dots-report-YYYYMMDD-HHMM.txt`, local time). What is copied or saved is the previewed text, byte for byte. No network sending.
4. **What the report holds:** the person's text (with secret-looking text scrubbed), the Dots version, OS and architecture, the engine kinds in use (`claude`, `local`: names only), the number of dots, the number of runs queued in the last 24 hours and how many of them failed, and, when asked for, the last 200 log lines.
5. **What it never holds:** chat messages, run payloads, summaries or errors, instructions, memories, dot names, project paths, vault contents, env values. None of these is read to build it; the only free text is the person's own and the log lines.

## 3. Log lines

- **Which:** the last 200 lines across the `dots.*.log` files in the log folder, newest file first; when the newest file has fewer, the tail of the one before it fills in, and so on. File names sort by date (`dots.YYYY-MM-DD.log`). A missing folder or unreadable file gives fewer lines, not an error.
- **What Dots logs (checked for this design, and again after a security review):** the log carries Dots' own operational lines (`target: message fields`). Messages, payloads and instructions are not logged, but some lines carry other text:
  - **Output of other programs, dropped and counted:** `dots_core::mcp::stdio` logs an MCP server's stderr (`MCP server stderr: …`); `dots_core::browser::host` logs the browser helper's stderr (a bare line with a `dot=` field). Continuation lines go with them. The report says `(N lines from MCP servers and the browser helper left out)`.
  - **Messages hidden** (the line keeps its time, level and target, then `[message hidden]`; its continuation lines become `[n more lines hidden]`), because the message can hold engine, tool, plugin or run text:
    - every target under `dots_core::engine` (Claude CLI notices `claude: …` at `engine/claude/mod.rs`, the prompt-write and request errors, the rate-limit debug JSON in `engine/claude/parse.rs`, the local engine's approval errors);
    - `dots_core::runner` lines starting `run failed:` (the error holds the engine's scrubbed stderr tail);
    - `dots_core::followups` lines starting `the follow-up's run could not be started`;
    - every `dots_core::library_service` line (plugin and hook warnings, `library_service.rs` 626–659);
    - every `dots_core::mcp::http` line (`describe` of an HTTP error can hold the server's answer);
    - every other `dots_core::browser::host` line (download file names, `host.rs` 276, 427, 439).
  - **Dot names:** `dots_core::scheduler` logs `dot = <name>` at many sites. Every current dot name (read from the store) becomes `[dot]`: longest first, whole words, any case. The log itself keeps them.
  - **Panics:** a panic's first line is kept and scrubbed; its continuation lines become `[n more lines hidden]`.
  - **Private keys:** from a `-----BEGIN` line through its `-----END` line, everything is dropped; the line that opens the block keeps what came before it, then `[private key hidden]`.
- **Scrubbing, every kept line, in order:**
  1. the process redactor again (`redact::global().scrub`), for secrets read since the line was written;
  2. secret patterns, each match replaced by `[secret]`: a private key line; URL credentials `://user:pw@` → `://[secret]@`; JWTs (`eyJ….….…`); the values of the query keys `sig`, `signature`, `code`, `key`, `auth`, `token`, `access_token`, `client_secret`, `password`, `pw`; `sk-ant-`/`sk-`/`sk_live_`/`ghp_`/`gho_`/`ghs_`/`github_pat_`/`xoxa-`/`xoxb-`/`xoxp-`/`xoxr-`/`AKIA`/`ASIA` keys; `Bearer <token>`; `Authorization: <anything>`; `password`/`passwd`/`secret`/`token`/`api_key` followed by `=` or `:` (the name is kept); a 32+ character hex or base64 run after the word key, secret or token;
  3. the dots' names → `[dot]`;
  4. paths → `[path]`, keeping only the file's extension (`[path].rs`): drive-letter paths in any slash form, debug-escaped, and with 8.3 short parts (`ASHAKU~1`); UNC paths; absolute paths under `/home`, `/Users`, `/root`. A space continues a path when the next word holds a slash (`My Projects\x`);
  5. email addresses, with or without a dot in the domain → `[email]` (no user name is left);
  6. what is left of the person's home folder (from `USERPROFILE`, else `HOME`) → `~`.
- The person's own text gets steps 1 and 2 (an email they type is theirs to share).

## 4. Format

```
Dots problem report
Made: 2026-11-03 14:05 (+05:30)

== What went wrong ==
<the person's text, or "(not described)">

== This copy of Dots ==
Version: 0.3.0
System: windows x86_64
Engines in use: claude, local
Dots: 4
Runs in the last 24 hours: 12 (2 failed)

== Recent log lines (200, scrubbed) ==
<lines, oldest first>
(3 lines from MCP servers and the browser helper left out)
```

When log lines are not included, the log section reads `== Recent log lines ==` / `(not included)`; with no log folder, `(no log found)`.

## 5. Core

New module `dots_core::report`:

- `pub struct ReportFacts { app_version, os, arch, engines: Vec<String>, dots: u32, runs_24h: u32, failed_24h: u32 }`.
- `Store::report_counts(since: DateTime<Utc>) -> Result<ReportCounts>` (distinct engines, dot count, runs with `queued_at >= since`, of those `status = 'failed'`), in a new `store/report.rs`.
- `report::Scrubber::new(dot_names, home)` and `Scrubber::line`; `Store::dot_names()`.
- `report::read_log_tail(dir: &Path, max: usize, scrubber: &Scrubber) -> LogSection` (tail, drop, hide, scrub, §3).
- `report::scrub_line(line, home) -> String`, `report::scrub_text(text) -> String`, `report::is_foreign_output(line) -> bool`.
- `report::render(facts, description, logs: &LogSection, made: DateTime<FixedOffset>) -> String`, `report::file_name(made)`.
- `report::build(store, app_version, log_dir: Option<&Path>, description, include_logs) -> Result<String>` (a free function over the `Store`, not a `Runtime` method: it needs nothing else, and tests need no runtime).

## 6. App

- Command `problem_report(description, includeLogs) -> String` (uses the app's version and the log folder from `AppState`).
- Command `save_problem_report(text) -> Option<String>`: the native save dialog from Rust (`tauri_plugin_dialog`, as `release_save_key` does), the text written to the chosen file; the path, or none when cancelled. The UI never names a path.
- `ui/src/api/commands.ts` and the contract fixture's `Commands` list (by hand).

## 7. UI

`ui/src/components/ReportProblemDialog.tsx` (new) and a Help section in `SettingsScreen.tsx`. Copy uses `navigator.clipboard`, as the other copy buttons do; success and failure are toasts.

## 8. Testing

- Core: the tail across files (short newest file, missing folder, `max`); scrubbing (each key style, bearer, Authorization header, password=, email, home path in both slash styles and cases, the redactor's registered secret); foreign lines dropped and counted; the render (sections, "not described", logs off, no log); `report_counts` (engines distinct, counts in and out of the window, failed only).
- App: the commands are registered.
- UI: the dialog shows the preview, refetches when the checkbox changes, copies the previewed text, saves through the command, and the Settings button opens it.

## 9. Not in this part

- Sending a report from inside Dots.
- Attaching the database or whole log files.
- A report from the fatal-error page (the runtime is not there; a later part can add a log-only report).
