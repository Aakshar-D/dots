# Chat-first home: smoke test

- Date: 2026-10-22
- Build: debug (`bun run tauri build --debug --no-bundle`), branch `feat/chat-home` at commit `6a8c76e` (Tasks 1 to 8 and the final review's fixes). Windows 11 on ARM (`aarch64-pc-windows-msvc`), Bun 1.4.3, LLVM for `ring`.
- Model: `qwen2.5:7b` in Ollama 0.40.7 at `http://127.0.0.1:11434`, for two local dots.
- Steps: 9, all as expected. 2 findings about the smoke tool, none about Dots.
- Screenshots: not in the repository; in the scratchpad folder of the run (`.../scratchpad/ch/shots`): `01-empty-home.png`, `02-chat-and-run-card.png`, `03-reply-under-card.png`, `04-older-topic.png`, `05-question-bar.png`, `06-inline-approval.png`.

## What this covers

Spec section 9's smoke: the home with and without dots, a local dot's chat, a run card and a reply to it, an approval answered inline, a question answered from the bar, New topic and History, the redirects from `/chat` and `/dots`, the Inbox's link, and the dot last opened. Run with the default reply target as decided after the final review (the current chat topic; a run card through its Reply).

## How it was run

- The app is `target/debug/dots-app.exe`, started with `DOTS_DATA_DIR` and `WEBVIEW2_USER_DATA_FOLDER` set to fresh folders in the run's scratchpad (`.../scratchpad/ch/data`, `.../ch/webview`), its `PATH` without the `claude` CLI (the app shows its "claude CLI was not found" banner). The real data folder was never used.
- The window is driven through WebView2's debugging port 9333 with `tools/smoke/webview.mjs` (`goto`, `click`, `fill`, `text`, `shot`, `eval`). Messages are typed into the composer and sent with its Send button, as a person does.
- Two local dots made with `create_dot` (`engine: 'local'`, `model: 'qwen2.5:7b'`, `workspace_mode: 'folder'`, `approval_wait_secs: 240`, preset `sandboxed`): `helper` (`01M5HVXCYST6A9WKGZA0TNN8KV`, "Answer the person in one short sentence.") and `asker` (`01M5HVXD7CVJ7YMT52XRT20FPY`, "Call the ask tool with the question Ship it? and the choices yes and no.").

## Steps

### 1. The home without dots

Expect: `/` opens `/home`; the rail (Home, Inbox, Library, Settings); "Create your first dot".

Result: pass. `#/home`, an empty Dots column with **+**, the text "A dot is an agent with instructions, a folder to work in, and a schedule or a webhook that starts it. You talk to it here." and the button "Create your first dot" (`01-empty-home.png`).

### 2. The home opens a dot

Expect: with dots, `/home` opens the first of the column (both never ran: by name).

Result: pass. After the two dots were made (by `invoke`, so the page was reloaded to see them), `/home` became `#/home/01M5HVXD7CVJ7YMT52XRT20FPY` (`asker`), with the header "asker · idle", History, New topic, and "Ask asker something. It answers with its own instructions, folder and permissions."

### 3. A chat

Expect: a message sent from the composer starts the dot's first topic; the reply shows in the conversation and the column's last line.

Result: pass. "What is 2 plus 2?" to `helper` made run `01M5HVZ6STVE0VGET6NZVHPFD4` (chat, no parent); the conversation showed it and "2 plus 2 is 4."; the column's row read "helper · 2 plus 2 is 4."; the chip "Replying to: current topic".

### 4. Run now from ⚙, and a reply to its card

Expect: Run now in the header's menu keeps the home and adds a run card; Reply on it sets the chip; the message continues that run; the chip goes back to the current topic.

Result: pass. Run now made run `01M5HW2FN7Z8453S3C9JCFGE68` (manual); the page stayed on `#/home/<helper>`; a card "Run now · 6:46 pm · Succeeded · Details · Reply" with its summary (`02-chat-and-run-card.png`). Reply set the chip to "Replying to: Run now · 6:46 pm"; "Say ok." made run `01M5HW37EZ0MTW5A16DM9Z6Y5T` with `parent_run_id` and `root_run_id` the manual run, shown under the card ("Ok."); the chip then read "Replying to: current topic" (`03-reply-under-card.png`). The menu items are `menuitem`s, which the smoke tool's `click` does not take; they were clicked with `eval` (finding 1).

### 5. New topic and History

Expect: New topic makes the next message start a topic of its own; History lists the earlier topics; one opens alone with the "Older topic" bar and replies to itself.

Result: pass. New topic set the chip to "New topic"; "Name a colour." made run `01M5HW3R0JT7K2C4FQKXQH823S` with no parent. History listed "Run now · 6:46 pm" and "What is 2 plus 2?", newest first; the second opened `#/home/<helper>/01M5HVZ6STVE0VGET6NZVHPFD4` with "Older topic · Back to the current conversation", that topic alone, and the chip "Replying to: What is 2 plus 2?" (`04-older-topic.png`).

### 6. A question, and the Inbox's link

Expect: the dot's question shows in its conversation and in the bar above the composer, the dot is "waiting for you"; the Inbox's Open conversation opens the dot; a choice in the bar answers it and continues the topic.

Result: pass. "Go." to `asker` (run `01M5HW4QDNM1F5E5VZVEWZRBHN`) asked "Ship it?" with yes and no; the header and the column said "waiting for you"; the bar "asker asked: Ship it?" with yes and no (`05-question-bar.png`). The rail's Inbox link was named "Inbox, 1 waiting". In the Inbox the message card had "Open conversation", which opened `#/home/01M5HVXD7CVJ7YMT52XRT20FPY`. "yes" in the bar made run `01M5HW5NM48465AWNS2M2DYBV5` with payload `{"answer": "yes", "message": "Answer to your question \"Ship it?\": yes"}`; the question was answered.

### 7. An approval answered inline

Expect: a call that asks shows its approval card in the conversation; the composer says "Waiting for your approval above"; approving there performs the call and the run continues.

Result: pass. "Use your shell tool to run the command: echo hi. …" (run `01M5HW66THK78T093VBJHRW45A`, sent to `asker`: see finding 2) parked on `Bash` `echo hi`; the conversation showed the card with Approve, Deny and "Approve and always allow", the status "Needs approval", and the disabled composer "Waiting for your approval above" (`06-inline-approval.png`). Approve: the approval `approved`, `apply_state` `applied`; the continuation said "The `echo hi` command has been run and printed "hi" to the console."; the composer was free again.

### 8. Redirects

Expect: `/dots` and `/chat` go to the home; `/chat/:dotId` to that dot; `/chat/:dotId/:runId` to the older topic of that run when it is not in the conversation; unknown addresses to the home.

Result: pass. `dots` → `#/home/<asker>` (the dot last opened then); `chat` → `#/home/<asker>`; `chat/<helper>` → `#/home/<helper>`; `chat/<helper>/01M5HVZ6STVE0VGET6NZVHPFD4` → `#/home/<helper>/01M5HVZ6STVE0VGET6NZVHPFD4`; `chat/<helper>/01M5HW37EZ0MTW5A16DM9Z6Y5T` (the reply to the manual run) → `#/home/<helper>/01M5HW2FN7Z8453S3C9JCFGE68`, its topic's root; `nowhere` → `#/home/<helper>`.

### 9. The dot last opened

Expect: a reload of `/home` opens the dot last opened.

Result: pass. With `helper` open last, a reload of `#/home` opened `#/home/01M5HVXCYST6A9WKGZA0TNN8KV`.

## Findings

1. `tools/smoke/webview.mjs` `click` takes buttons, tabs, switches and links, not menu items (`role="menuitem"`): the ⚙ menu's and History's items were clicked with `eval` (`[role=menuitem]` by text, `.click()`). A smoke tool limit, not a Dots defect.
2. `click helper` matched no control once the row's accessible name had become "helper Sure, my favorite color is blue." (the link holds the last line), so the next message went to the dot still open (`asker`). As the person sees it the row is right; the smoke tool needs the exact name. Not a Dots defect.
