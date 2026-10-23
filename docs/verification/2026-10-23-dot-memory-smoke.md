# Memory for each dot: smoke test

- Date: 2026-10-23
- Build: debug (`bun run tauri build --debug --no-bundle`), branch `feat/dot-memory` after Task 7 (`a75c4b3`). Windows 11 on ARM, Bun 1.4.3.
- Data: a fresh sandbox data folder (`.../scratchpad/mem/data`), one local dot "keeper" on `qwen2.5:7b` (Ollama).
- Steps: 4, all as expected.
- Screenshots: not in the repository; `.../scratchpad/mem/shots/memory-tab.png`.

## How it was run

The app was started on the data folder above with WebView2's debugging port 9333 and driven with `tools/smoke/webview.mjs` and the CDP helper of the earlier smokes (`scratchpad/qa/cdp.mjs`). Prompts were read from the run records in the data folder.

## Steps

### 1. The dot saves what it is told to remember

Expect: told "remember I prefer invoices as PDF", the dot calls `remember`; the memory is stored with source `dot`.

Result: pass. The memory was stored with source `dot`; the conversation shows "Remembered: I prefer invoices as PDF." under the tool chip.

### 2. A later run knows it

Expect: a new topic (no parent run) asked about invoices answers from memory; its prompt holds the "What you remember" section.

Result: pass. The dot answered that the person prefers invoices as PDF without calling a tool; the recorded prompt contains `What you remember`.

### 3. The person adds, pins and deletes memories

Expect: `?tab=memory` opens the tab; Add memory saves a memory "by you"; a memory can be pinned; Delete asks "Delete this memory?" and removes it.

Result: pass. "My accountant is Priya…" was added "by you"; a memory was pinned; Delete showed "Delete this memory?" and removed it.

### 4. A secret is refused

Expect: adding a text with an `sk-` key is refused with the spec's sentence.

Result: pass. The tab showed "That looks like a secret. Don't store secrets in memory; ask the person to keep it in the vault."

## Findings

- Minor: two buttons are named "Save" when the Add box and an Edit box are open together (deferred, listed in the PR).
