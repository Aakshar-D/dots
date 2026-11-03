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
- **What Dots logs (checked for this design):** the log carries Dots' own operational lines (`target: message fields`); messages, payloads and instructions are not logged. Three places write text that Dots does not control:
  - `dots_core::mcp::stdio` logs an MCP server's stderr (`MCP server stderr: …`);
  - `dots_core::browser::host` logs the browser helper's stderr (a bare `WARN` line with a `dot=` field);
  - `dots_core::engine::claude` logs Claude CLI notices (`claude: …`, plugin load errors).
  A server or helper can echo what it was given, so the first two are **dropped** from the report. Claude notices are kept (they are Dots' plugin errors, and are scrubbed like every line). Dropped lines are counted: the report says `(N lines from MCP servers and the browser helper left out)`.
- **Scrubbing, every kept line, in order:**
  1. the process redactor again (`redact::global().scrub`), for secrets read since the line was written;
  2. secret patterns, each match replaced by `[secret]`: private key blocks, `sk-`/`sk_live_`/`ghp_`/`gho_`/`github_pat_`/`xoxb-`/`xoxp-`/`AKIA` keys, `Bearer <token>`, `Authorization: <anything>`, and `password`/`passwd`/`secret`/`token`/`api_key` followed by `=` or `:` (the name is kept, the value replaced);
  3. email addresses → `[email]`;
  4. the person's home folder (from `USERPROFILE`, else `HOME`), in either slash direction and any letter case → `~`.
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
- `Store::report_counts(since: DateTime<Utc>) -> Result<(Vec<String>, u32, u32, u32)>` style query (distinct engines, dot count, runs with `queued_at >= since`, of those `status = 'failed'`), in a new `store/report.rs`.
- `report::recent_log_lines(dir: &Path, max: usize) -> Vec<String>` (raw tail, §3).
- `report::scrub_line(line: &str, home: Option<&str>) -> String`, `report::is_foreign_output(line) -> bool`.
- `report::render(facts, description, logs: Option<LogTail>, made: DateTime<FixedOffset>) -> String`.
- `Runtime::problem_report(app_version, log_dir: Option<&Path>, description, include_logs) -> Result<String>`.

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
