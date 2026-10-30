# Conversational Chat, Plan B (the conversation layout) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A dot's conversation takes the Claude app's layout. It is one centred column with the person's messages in bubbles and the dot's replies as plain text, tool steps folded into one line, and a rounded composer. The header names the dot and opens its menu, and an empty conversation greets with the dot's name. The colours stay Dots' own.

**Architecture:**
- **Transcript:** `components/Transcript.tsx` gains `variant: "run" | "conversation"`. The run screen keeps the default, unchanged. The home's `TopicRuns` passes `"conversation"`, which drops the meta lines of chat runs and folds each group of tool steps into one line (`lib/transcript.ts` `stepsLine`) that opens to today's chips.
- **Column and composer:** `Timeline` and `ReplyArea` share one column at most 720px wide. The `Composer` is one rounded box with a textarea that grows with its text and a round Send button.
- **Header and column:** `DotHeader` holds the dot's name as `DotMenu`'s trigger, and the ⚙ goes. `DotsColumn` starts with a "New dot" row.
- **Empty and cards:** `DotPane` shows the greeting in place of the timeline. `RunCard` becomes a rounded card. `InstructionsCard` takes `busy`, which stops a decision while its topic works.

**Tech Stack:** Bun 1.3, React 19.3, TypeScript 6.0, Tailwind 4.3 with the shadcn theme tokens, lucide-react, radix-ui, Vitest 5.0, Testing Library. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-27-conversational-chat-design.md` (section 5, the layout; section 7, its UI tests; section 9, what plan A built), at `b7df4ea`, the base of this plan: `origin/master` after PR #33 (plan A). Task 6 adds "Plan B (section 5)" to its section 9, which the table "Deviations" lists.

**Where the tasks come from:** spec section 5:
- Task 1: the dot's replies and the tool steps
- Task 2: the reading column and the composer
- Task 3: the header and the Dots column
- Task 4: the empty conversation
- Task 5: the cards, and section 9's known gap

Each task's tests come from section 7.

**How this plan's code was checked:** the code blocks of Tasks 1 to 6 were generated from a throwaway worktree of `b7df4ea`, one commit per task, where they passed on Windows 11. The plan was then replayed onto `b7df4ea` by a script that applied every block and compared the tree with each task commit. All 6 tasks matched. Each task's new tests were run against the code before it and failed as Step 2 says.

| | Before the first task | At the last task |
|---|---|---|
| `bun run --cwd ui test --maxWorkers=2` | 840 tests in 84 files | 857 in 84 |
| `bun run --cwd ui typecheck` | clean | clean |

No Rust file changes, so no cargo command is needed. No agent ran the app or the real `claude` CLI.

## Global Constraints

- Toolchain: Bun 1.3 (`bun x`, not `bunx`, which is not on the PATH in Git Bash). Windows 11, Git Bash.
- Branch `feat/conversational-chat-b`, in the worktree `C:\Users\aksha\dots\.claude\worktrees\conversational-chat-b`, based on `b7df4ea`. All commands run from the repository root unless a step says otherwise.
- Never commit to `master`.
- `git add` names its files: never `git add -A` or `git add .` (a `.memsearch/` folder in a worktree is not ours).
- In a fresh worktree, run `bun install --cwd ui --frozen-lockfile` first.
- UI only. This plan adds no Rust change, no contract or type change, no dependency, no route, no colour token and no Tauri capability. Every look uses the existing theme tokens (`bg-card`, `border`, `text-muted-foreground`, `bg-bubble`, `text-needs`, `text-destructive`, …), so light and dark both work.
- The run screen (`screens/RunScreen.tsx`) must look as before: it renders `Transcript` without `variant`.
- Nothing the home does today is lost: approvals, questions, run cards, the reply target, History, New topic, the locked composer, errors kept in the composer, the older-topic line (spec section 2, anti-goals).
- The subscription rule: Dots only spawns the official `claude` CLI and never reads, copies or sends its credentials. No agent runs the real `claude` CLI.
- No agent installs or runs an installer, runs a release build, or touches the installed Dots (`%LOCALAPPDATA%\Dots`) or the real data folder `%LOCALAPPDATA%\dots`.
- Values this plan fixes:
  - **The column:** `max-w-[720px]`, centred (`mx-auto w-full`), `px-4` side padding.
  - **The bubble:** at most `max-w-[75%]` in the conversation (the run screen keeps 80%).
  - **The composer:** the textarea grows with `field-sizing-content` up to `max-h-40`, then scrolls. The send button is named "Send".
  - **The steps line:** Task 1 fixes its copy (`stepsLine`), and adds " · N failed", " · waiting for approval" and " · running…".
  - **The greeting:** the dot's name and "Ask <dot> something".
  - **The busy card:** `title="<dot> is still working on this topic"`.
- How the code blocks are written. A step changes files in two ways, and its blocks are applied in the order given:
  - **Create `path`:** the block is the whole new file.
  - **In `path`, replace:** … **with:** … The first block is a passage of the file as it stands at that moment, and it occurs there exactly once; the second block takes its place. Lines above and below the change are in both blocks, so that the place is unambiguous.
- A step that says "replace" must find its passage once. If it does not (an earlier fix round changed the file), stop and compare with the prototype's intent before changing anything; do not paste over a fix.
- Files are LF with a final newline, and valid UTF-8. Before each commit, check with:
  - `git diff --cached --check`
  - `git ls-files --eol <files>`

  The blocks hold `·`, `…`, `↓` and `›`. The working copies may be CRLF (`core.autocrlf`); the index must hold LF.
- The UI files have no formatter; keep new lines within 100 columns where the file does.
- Before each commit, run the task's own tests. Then run:
  - `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test --maxWorkers=2`
  - `bun run --cwd ui typecheck`
- Known on this machine:
  - A full `vitest` run without `--maxWorkers=2` runs out of memory. It fails with "[vitest-pool]: Failed to start forks worker … Timeout waiting for worker to respond" and reports fewer than 84 files. With `--maxWorkers=2` it can still happen; rerun it on its own.
  - Before trusting a count, check that it says 84 files.
- Fact-Forcing Gate (a hook of this machine): these are blocked until the facts it asks for are stated, after which you retry the identical call:
  - the first Edit or Write of each file
  - the first Bash command after a task notification
  - `rm`, `git checkout -f`, `git commit --amend`, or a rebase

## Deviations from the spec

The spec at `b7df4ea` leaves these open; Task 6 records them in section 9.

| Spec at `b7df4ea` | This plan | Why |
|---|---|---|
| Section 5: tool steps as one line, "▸ Read 2 files · Ran a command" | `stepsLine` names each engine's built-in tools by what they did, and any other tool "Used <its chip's label>". The line adds " · N failed", " · waiting for approval" and " · running…". The memory notes stay under the line, always shown | A folded line must not hide a failure or a call waiting for the person |
| Section 5: the dot's replies have "no name label" | A chat run also loses its header line ("Chat message · time"); runs of other triggers in a topic keep theirs, and every run's section keeps its accessible name | In a conversation the bubble says who wrote it; a resume after an approval still says so |
| Section 5: the header | The visible status words go (the mark keeps its name and tooltip); the menu opens at the name's left edge (`align="start"`) | The header is slim; the trigger moved from the right to the left |
| Section 5: run cards, approvals, questions, check-ins, browser calls and failures are restyled to rounded light-bordered cards | Only `RunCard` changes. `ApprovalCard`, `InstructionsCard` and `AppReviewCard` were already rounded bordered cards. Check-ins and a run card's question stay soft muted cards, `BrowserCall` a rounded bordered line, and `Failure` its red rule | `Failure` and `CheckIn` are shared with the run screen, which must not change; `AppReviewCard` is being changed on `feat/app-library-c` |
| Section 9: "Known gap", Approve and Reject enabled while the topic works | In the conversation the card is disabled while a run of its topic is active. The Inbox's card is not, and the core still refuses there | The Inbox does not load the topic's runs |

## Review Focus

The inputs and conditions most likely to bite a person that the spec implies, each pinned to the test that covers it:

1. **A step that failed, or a call waiting for approval, inside a folded steps line.** Expected: the line says " · 1 failed" or " · waiting for approval" while closed. Tests: `Document.test.tsx` "says in the steps line that a step failed or waits for approval", and `Conversation.test.tsx` "says a call parked for approval was sent for approval, not that it failed" (Task 1).
2. **The run screen after the conversation's restyle.** Expected: its chips, run header, "<dot> · time" line and 80% bubble are as before, with no steps line. Test: `RunScreen.test.tsx` "shows a chat run's tool steps as chips, with its header and the dot's name label" (Task 1).
3. **The first message sent from the greeting.** Expected: the send completes, the composer is not mounted again mid-send, and the conversation replaces the greeting. Tests: `Conversation.test.tsx` "turns the greeting into the conversation with the first message" (Task 4), and `Sending.test.tsx` "starts the first topic of a dot with its first message" (unchanged apart from `findByRole`).
4. **Approving a proposal while the dot still works on its topic.** Expected: Approve and Reject are disabled with the busy title, and enabled once the run ends. Test: `Instructions.test.tsx` "waits to be decided while the dot still works on its topic" (Task 5).
5. **Reaching the dot's menu without the ⚙ button.** Expected: the name opens it by mouse and by keyboard, and Memory… still opens the editor's Memory tab. Tests: `HomeMenu.test.tsx` "opens from the keyboard", and `MemoryTab.test.tsx` "opens from the dot's menu" (Task 3).

## File Map

- `ui/src/lib/transcript.ts`, `ui/src/components/Transcript.tsx`, `ui/src/screens/home/TopicRuns.tsx` (Tasks 1, 5).
- `ui/src/screens/home/{Timeline,ReplyArea,Composer}.tsx` (Task 2).
- `ui/src/screens/home/{DotHeader,DotMenu,DotsColumn}.tsx` (Task 3), `DotPane.tsx` (Tasks 3, 4).
- `ui/src/screens/home/RunCard.tsx`, `ui/src/components/InstructionsCard.tsx` (Task 5).
- Tests:
  - `ui/src/lib/transcript.test.ts`, `ui/src/screens/RunScreen.test.tsx`, `ui/src/screens/home/{Document,MemoryNote}.test.tsx` (Task 1)
  - `Conversation.test.tsx` (Tasks 1, 4, 5)
  - `Sending.test.tsx` (Tasks 2, 4)
  - `HomeMenu.test.tsx`, `HomeColumn.test.tsx`, `ui/src/lib/query.test.tsx`, `ui/src/screens/DotOrgs.test.tsx`, `ui/src/screens/dot-editor/MemoryTab.test.tsx` (Task 3, which moves them from the ⚙ to the name)
  - `Instructions.test.tsx` (Task 5)
- `docs/superpowers/specs/2026-10-27-conversational-chat-design.md` (Task 6).

### Task 1: The dot's replies read as plain text, and tool steps fold into one line

Spec section 5, "The dot's replies" and "Tool steps": in the conversation a chat run shows no header or name label, and each group of consecutive tool steps is one muted line that opens to today's chips. The run screen is unchanged.

**Files:**

- Modify: `ui/src/lib/transcript.ts`, `ui/src/components/Transcript.tsx`, `ui/src/screens/home/TopicRuns.tsx`
- Test: `ui/src/lib/transcript.test.ts`, `ui/src/screens/RunScreen.test.tsx`, `ui/src/screens/home/Document.test.tsx`, `ui/src/screens/home/MemoryNote.test.tsx`, `ui/src/screens/home/Conversation.test.tsx`

**Interfaces:**

- Produces: `stepsLine(items: { tool: string; input: unknown }[], owners: ReadonlyMap<string, string>): string` in `lib/transcript.ts`; `Transcript`'s prop `variant?: "run" | "conversation"` (default `"run"`). The steps line is `<details data-slot="steps">`; opened, it holds the chip group `role="group"` named "Tool steps", as before.

- [ ] **Step 1: Write the failing tests**

**In `ui/src/lib/transcript.test.ts`, replace:**

```ts
import type { RunEventRecord } from "@/api/types";
import { toolPreview, transcriptItems } from "@/lib/transcript";

```

**with:**

```ts
import type { RunEventRecord } from "@/api/types";
import { stepsLine, toolPreview, transcriptItems } from "@/lib/transcript";

```

**In `ui/src/lib/transcript.test.ts`, replace:**

```ts
  });
});
```

**with:**

```ts
  });
});

describe("the line a group of tool steps shows as", () => {
  const none = new Map<string, string>();
  const step = (tool: string, input: unknown = {}) => ({ tool, input });

  it("counts the steps by phrase, in the order each phrase first comes", () => {
    expect(
      stepsLine([step("Read"), step("Bash"), step("read_file"), step("Grep")], none),
    ).toBe("Read 2 files · Ran a command · Searched the files");
    expect(stepsLine([step("WebSearch"), step("Edit"), step("WebFetch")], none)).toBe(
      "Searched the web · Edited a file · Read a web page",
    );
  });

  it("says how many for each phrase", () => {
    expect(stepsLine([step("Write"), step("MultiEdit"), step("edit_file")], none)).toBe(
      "Edited 3 files",
    );
    expect(stepsLine([step("Bash"), step("shell")], none)).toBe("Ran 2 commands");
    expect(stepsLine([step("Glob"), step("LS"), step("list_dir")], none)).toBe(
      "Searched the files 3 times",
    );
    expect(stepsLine([step("WebFetch"), step("WebFetch")], none)).toBe("Read 2 web pages");
    expect(stepsLine([step("WebSearch"), step("WebSearch")], none)).toBe(
      "Searched the web 2 times",
    );
    expect(stepsLine([step("Read"), step("Read")], none)).toBe("Read 2 files");
  });

  it("names any other tool by the label of its chip, one phrase per tool", () => {
    const owners = new Map([["sf", "Salesforce"]]);
    expect(
      stepsLine(
        [
          step("mcp__sf__query"),
          step("mcp__github__search"),
          step("mcp__sf__query"),
          step("mcp__sf__query"),
        ],
        owners,
      ),
    ).toBe("Used Salesforce › query 3 times · Used github › search");
    expect(stepsLine([step("Task")], none)).toBe("Used Task");
  });

  it("names a local memory call by the tool it ran", () => {
    expect(
      stepsLine(
        [
          step("Memory", { text: "A fact." }),
          step("Memory", { query: "x" }),
          step("Memory", { text: "B." }),
        ],
        none,
      ),
    ).toBe("Used Memory › remember 2 times · Used Memory › recall");
    expect(stepsLine([step("mcp__dots__remember", { text: "A fact." })], none)).toBe(
      "Used dots › remember",
    );
  });
});
```

**In `ui/src/screens/RunScreen.test.tsx`, replace:**

```tsx
import type { ApplyState, Approval, Run, RunEventRecord, WorktreeDiff } from "@/api/types";
import { localDateTime } from "@/lib/format";
import { backend, later } from "@/test/backend";
```

**with:**

```tsx
import type { ApplyState, Approval, Run, RunEventRecord, WorktreeDiff } from "@/api/types";
import { localDateTime, localTime } from "@/lib/format";
import { backend, later } from "@/test/backend";
```

**In `ui/src/screens/RunScreen.test.tsx`, replace:**

```tsx
    expect(screen.getByText("Unparsed output").closest("details")).not.toHaveAttribute("open");
  });
```

**with:**

```tsx
    expect(screen.getByText("Unparsed output").closest("details")).not.toHaveAttribute("open");
  });

  it("shows a chat run's tool steps as chips, with its header and the dot's name label", async () => {
    const asked: Run = { ...folderRun, trigger: "chat", payload: { message: "Run the tests." } };
    thread(
      [asked],
      [
        ev({ kind: "tool_call", id: "t1", tool: "Read", input: { file_path: "a.rs" } }),
        ev({ kind: "tool_result", id: "t1", output: "fn a", is_error: false }),
        ev({ kind: "tool_call", id: "t2", tool: "Bash", input: { command: "cargo test" } }),
        ev({ kind: "tool_result", id: "t2", output: "ok", is_error: false }),
        ev({ kind: "assistant_text", text: "The tests pass." }),
      ],
    );
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByText("The tests pass.")).toBeVisible();
    // The conversation's look is the home's: the run screen keeps its own.
    const chips = screen.getByRole("group", { name: "Tool steps" });
    expect(within(chips).getByText("Read")).toBeVisible();
    expect(within(chips).getByText("Bash")).toBeVisible();
    expect(chips.closest("details")).toBeNull();
    expect(screen.queryByText("Read a file · Ran a command")).toBeNull();
    expect(screen.getByText("Chat message")).toBeVisible();
    expect(screen.getByText(`${sample.dot.name} · ${localTime(asked.queued_at)}`)).toBeVisible();
    expect(screen.getByText("Run the tests.")).toHaveClass("max-w-[80%]");
  });
```

**In `ui/src/screens/home/Conversation.test.tsx`, replace:**

```tsx
    );
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByText("sent for approval")).toBeVisible();
    expect(view.queryByText("failed")).toBeNull();
```

**with:**

```tsx
    );
    const { user } = renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    // The chip is under the steps line.
    await user.click(await view.findByText("Ran a command"));
    expect(view.getByText("sent for approval")).toBeVisible();
    expect(view.queryByText("failed")).toBeNull();
```

**In `ui/src/screens/home/Document.test.tsx`, replace:**

```tsx
describe("the conversation as a document (deep-teal spec section 4)", () => {
  it("shows the person's message as a bubble, then the dot's words under a meta line", async () => {
    const a = chat("a", "10:00");
```

**with:**

```tsx
describe("the conversation as a document (deep-teal spec section 4)", () => {
  it("shows the person's message as a bubble, then the dot's words with no label", async () => {
    const a = chat("a", "10:00");
```

**In `ui/src/screens/home/Document.test.tsx`, replace:**

```tsx
    expect(bubble).toHaveAttribute("data-from", "person");
    const meta = view.getByText(`${dot.name} · ${localTime(a.queued_at)}`);
    expect(follows(bubble, meta)).toBe(true);
    expect(follows(meta, words)).toBe(true);
  });
```

**with:**

```tsx
    expect(bubble).toHaveAttribute("data-from", "person");
    expect(bubble).toHaveClass("max-w-[75%]", "break-words");
    expect(follows(bubble, words)).toBe(true);
    // A person's message has no header line, and the dot's words no name label; the run's
    // section keeps its name.
    expect(view.queryByText("Chat message")).toBeNull();
    expect(view.queryByText(`${dot.name} · ${localTime(a.queued_at)}`)).toBeNull();
    expect(view.getByRole("region", { name: /^Chat message, / })).toContainElement(bubble);
  });

  it("keeps the header line of a run another trigger started", async () => {
    const resumed = chat("b", "10:05", { trigger: "resume", root_run_id: "a", parent_run_id: "a" });
    home([chat("a", "10:00"), resumed], [said("b", 1, "Pushed.")]);
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    await view.findByText("Pushed.");
    expect(view.getByText("Continued after approval")).toBeVisible();
    expect(view.queryByText("Chat message")).toBeNull();
  });

  it("shows a group of tool steps as one line that opens to their chips", async () => {
    home(
      [chat("a", "10:00")],
      [
        event("a", 1, { kind: "tool_call", id: "t1", tool: "Read", input: { file_path: "a.rs" } }),
        event("a", 2, { kind: "tool_result", id: "t1", output: "fn a", is_error: false }),
        event("a", 3, { kind: "tool_call", id: "t2", tool: "Bash", input: { command: "cargo test" } }),
        event("a", 4, { kind: "tool_result", id: "t2", output: "ok", is_error: false }),
        event("a", 5, { kind: "tool_call", id: "t3", tool: "Read", input: { file_path: "b.rs" } }),
        event("a", 6, { kind: "tool_result", id: "t3", output: "fn b", is_error: false }),
        event("a", 7, { kind: "assistant_text", text: "All tests pass." }),
      ],
    );
    const { user } = renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const line = await view.findByText("Read 2 files · Ran a command");
    expect(line).toBeVisible();
    const steps = line.closest("details")!;
    expect(steps).toHaveAttribute("data-slot", "steps");
    expect(steps.querySelector("summary")).toHaveClass("text-xs", "text-muted-foreground");
    expect(within(steps).getByRole("group", { name: "Tool steps" })).not.toBeVisible();
    expect(within(steps.querySelector("summary")!).queryByText(/failed|waiting|running/)).toBeNull();

    await user.click(line);
    expect(steps).toHaveAttribute("open");
    const chips = within(steps).getByRole("group", { name: "Tool steps" });
    expect(chips).toBeVisible();
    expect(within(chips).getAllByText("Read")).toHaveLength(2);
    expect(within(chips).getByText("Bash")).toBeVisible();
  });

  it("says in the steps line that a step failed or waits for approval", async () => {
    home(
      [chat("a", "10:00", { status: "awaiting_approval", ended_at: null })],
      [
        event("a", 1, { kind: "tool_call", id: "t1", tool: "Read", input: { file_path: "a.rs" } }),
        event("a", 2, { kind: "tool_result", id: "t1", output: "no such file", is_error: true }),
        event("a", 3, { kind: "tool_call", id: "t2", tool: "Bash", input: { command: "git push" } }),
        event("a", 4, {
          kind: "tool_result",
          id: "t2",
          output: "Queued for human approval #01APPROVAL. Do not retry this action.",
          is_error: true,
        }),
      ],
    );
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const line = await view.findByText("Read a file · Ran a command");
    const summary = within(line.closest("summary")!);
    // The parked call waits: only the read counts as failed.
    expect(summary.getByText("· 1 failed")).toHaveClass("text-destructive");
    expect(summary.getByText("· waiting for approval")).toHaveClass("text-needs");
    expect(summary.queryByText(/running/)).toBeNull();
  });

  it("says in the steps line that a step is running while its run goes on", async () => {
    home(
      [chat("a", "10:00", { status: "running", ended_at: null })],
      [event("a", 1, { kind: "tool_call", id: "t1", tool: "Bash", input: { command: "cargo test" } })],
    );
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const line = await view.findByText("Ran a command");
    expect(within(line.closest("summary")!).getByText("· running…")).toBeVisible();
  });
```

**In `ui/src/screens/home/Document.test.tsx`, replace:**

```tsx
    const view = await conversation();
    const steps = await view.findByRole("group", { name: "Tool steps" });
    const [bash, read] = [...steps.querySelectorAll("details")];
```

**with:**

```tsx
    const view = await conversation();
    // The chips are under the steps line.
    await user.click(await view.findByText("Ran a command · Read a file"));
    const steps = view.getByRole("group", { name: "Tool steps" });
    const [bash, read] = [...steps.querySelectorAll("details")];
```

**In `ui/src/screens/home/MemoryNote.test.tsx`, replace:**

```tsx
      ],
    );
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    const remembered = await view.findByText("Remembered: Prefers invoices as PDF.");
```

**with:**

```tsx
      ],
    );
    const { user } = renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    const remembered = await view.findByText("Remembered: Prefers invoices as PDF.");
```

**In `ui/src/screens/home/MemoryNote.test.tsx`, replace:**

```tsx
    const forgot = view.getByText("Forgot a memory");
    // Each chip shows as every tool's does; the notes follow the row of chips.
    const chips = view.getByRole("group", { name: "Tool steps" });
```

**with:**

```tsx
    const forgot = view.getByText("Forgot a memory");
    // The notes show while the steps line is closed.
    expect(remembered).toBeVisible();
    expect(forgot).toBeVisible();
    // Each chip shows as every tool's does; the notes follow the row of chips.
    await user.click(
      view.getByText("Used Memory › remember 2 times · Used dots › forget · Used dots › recall"),
    );
    const chips = view.getByRole("group", { name: "Tool steps" });
```

**In `ui/src/screens/home/MemoryNote.test.tsx`, replace:**

```tsx
    );
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    const chips = await view.findByRole("group", { name: "Tool steps" });
    for (const name of ["Memory › remember", "Memory › forget", "Memory › recall"]) {
```

**with:**

```tsx
    );
    const { user } = renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    const line = await view.findByText(
      "Used Memory › remember · Used Memory › forget · Used Memory › recall",
    );
    await user.click(line);
    const chips = view.getByRole("group", { name: "Tool steps" });
    for (const name of ["Memory › remember", "Memory › forget", "Memory › recall"]) {
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test --maxWorkers=2 src/lib/transcript.test.ts src/screens/RunScreen.test.tsx src/screens/home/Conversation.test.tsx src/screens/home/Document.test.tsx src/screens/home/MemoryNote.test.tsx`
Expected: FAIL, 13 failed and 73 passed (86): `stepsLine` is not a function, and no steps line in the conversation.

- [ ] **Step 3: Give the transcript its conversation look**

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
import { cn } from "cn";
import { Brain, Info } from "lucide-react";
import { Link } from "react-router";
```

**with:**

```tsx
import { cn } from "cn";
import { Brain, ChevronRight, Info } from "lucide-react";
import { Link } from "react-router";
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
  memoryNote,
  toolPreview,
```

**with:**

```tsx
  memoryNote,
  stepsLine,
  toolPreview,
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx

/// What a run's section shows, in order: consecutive tool steps as one row of chips, every
/// other item by itself.
type Block =
```

**with:**

```tsx

/// One row of tool step chips.
function ToolChips(props: {
  items: ToolItem[];
  active: boolean;
  owners: ReadonlyMap<string, string>;
  className?: string;
}) {
  return (
    <div
      role="group"
      aria-label="Tool steps"
      className={cn("flex flex-wrap items-start gap-1.5", props.className)}
    >
      {props.items.map((item) => (
        <ToolChip key={item.seq} item={item} active={props.active} owners={props.owners} />
      ))}
    </div>
  );
}

/// A group of consecutive tool steps in the conversation (conversational chat spec section 5):
/// one muted line that says what they did and how it went, and opens to their row of chips. A
/// call parked for approval is not counted as failed: it waits.
function StepsLine(props: {
  items: ToolItem[];
  active: boolean;
  owners: ReadonlyMap<string, string>;
}) {
  const { items, active, owners } = props;
  const failed = items.filter((item) => item.result?.isError && !parked(item.result)).length;
  const waiting = items.some((item) => parked(item.result));
  const running = active && items.some((item) => item.result === null);
  return (
    <details data-slot="steps" className="group/steps">
      <summary className="flex cursor-pointer list-none items-center gap-1 text-xs text-muted-foreground [&::-webkit-details-marker]:hidden">
        <ChevronRight
          aria-hidden
          className="size-3.5 shrink-0 transition-transform group-open/steps:rotate-90 motion-reduce:transition-none"
        />
        <span className="min-w-0 truncate">{stepsLine(items, owners)}</span>
        {failed > 0 && <span className="shrink-0 text-destructive"> · {failed} failed</span>}
        {waiting && <span className="shrink-0 text-needs"> · waiting for approval</span>}
        {running && <span className="shrink-0"> · running…</span>}
      </summary>
      <ToolChips items={items} active={active} owners={owners} className="mt-1.5" />
    </details>
  );
}

/// What a run's section shows, in order: consecutive tool steps as one row of chips (one line in
/// the conversation), every other item by itself.
type Block =
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
  shownError?: string | null;
  /// The dot's name, for the meta line above its words.
  dotName?: string;
}) {
  const many = props.runs.length > 1;
```

**with:**

```tsx
  shownError?: string | null;
  /// The dot's name, for the meta line above its words in the run variant.
  dotName?: string;
  /// "run" (the default) is the run screen's look. "conversation" is the home's (conversational
  /// chat spec section 5): the dot's words with no meta line, a person's message with no header
  /// line, and each group of tool steps as one line that opens to its chips.
  variant?: "run" | "conversation";
}) {
  const conversation = props.variant === "conversation";
  const many = props.runs.length > 1;
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
          <section key={run.id} aria-label={names[i]}>
            <header className="mb-2 flex items-center gap-2 text-xs text-muted-foreground">
              <span>{runTrigger(run)}</span>
              <span>{localDateTime(run.queued_at)}</span>
              {many && <StatusBadge status={run.status} />}
            </header>
            <ScheduleLine run={run} />
```

**with:**

```tsx
          <section key={run.id} aria-label={names[i]}>
            {/* In the conversation a person's message needs no header: its bubble says it. The
                section keeps its name either way. */}
            {!(conversation && run.trigger === "chat") && (
              <header className="mb-2 flex items-center gap-2 text-xs text-muted-foreground">
                <span>{runTrigger(run)}</span>
                <span>{localDateTime(run.queued_at)}</span>
                {many && <StatusBadge status={run.status} />}
              </header>
            )}
            <ScheduleLine run={run} />
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
                  data-from="person"
                  className="max-w-[80%] self-end rounded-2xl rounded-br-sm bg-bubble px-3.5 py-2 whitespace-pre-wrap text-bubble-foreground"
                >
```

**with:**

```tsx
                  data-from="person"
                  className={cn(
                    "self-end rounded-2xl rounded-br-sm bg-bubble px-3.5 py-2 whitespace-pre-wrap text-bubble-foreground",
                    conversation ? "max-w-[75%] break-words" : "max-w-[80%]",
                  )}
                >
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
              )}
              {(items.length > 0 || checkIns.length > 0) && (
                <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
```

**with:**

```tsx
              )}
              {!conversation && (items.length > 0 || checkIns.length > 0) && (
                <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
```

**In `ui/src/components/Transcript.tsx`, replace:**

```tsx
                  <Fragment key={block.seq}>
                    <div
                      role="group"
                      aria-label="Tool steps"
                      className="flex flex-wrap items-start gap-1.5"
                    >
                      {block.items.map((item) => (
                        <ToolChip
                          key={item.seq}
                          item={item}
                          active={isActive(run)}
                          owners={owners}
                        />
                      ))}
                    </div>
                    {block.items.map((item) => (
```

**with:**

```tsx
                  <Fragment key={block.seq}>
                    {conversation ? (
                      <StepsLine items={block.items} active={isActive(run)} owners={owners} />
                    ) : (
                      <ToolChips items={block.items} active={isActive(run)} owners={owners} />
                    )}
                    {/* The memory notes stay under the steps, shown while the steps are closed. */}
                    {block.items.map((item) => (
```

**In `ui/src/lib/transcript.ts`, replace:**

```ts
import { BROWSER_PREFIX } from "@/lib/browser";

```

**with:**

```ts
import { BROWSER_PREFIX } from "@/lib/browser";
import { mcpToolLabel } from "@/lib/mcp-tool";

```

**In `ui/src/lib/transcript.ts`, replace:**

```ts
    : { kind: "forgot" };
}
```

**with:**

```ts
    : { kind: "forgot" };
}

/// How a group of tool steps reads, for one step and for `n` of them.
type StepPhrase = { one: string; many: (n: number) => string };

const READ_FILE: StepPhrase = { one: "Read a file", many: (n) => `Read ${n} files` };
const EDIT_FILE: StepPhrase = { one: "Edited a file", many: (n) => `Edited ${n} files` };
const COMMAND: StepPhrase = { one: "Ran a command", many: (n) => `Ran ${n} commands` };
const SEARCH_FILES: StepPhrase = {
  one: "Searched the files",
  many: (n) => `Searched the files ${n} times`,
};
const WEB_PAGE: StepPhrase = { one: "Read a web page", many: (n) => `Read ${n} web pages` };
const WEB_SEARCH: StepPhrase = {
  one: "Searched the web",
  many: (n) => `Searched the web ${n} times`,
};

/// The tools a phrase of its own says, by the names both engines store their calls under: the
/// Claude engine's built-in tools and the local engine's.
const STEP_PHRASES = new Map<string, StepPhrase>([
  ["Read", READ_FILE],
  ["read_file", READ_FILE],
  ["Write", EDIT_FILE],
  ["Edit", EDIT_FILE],
  ["MultiEdit", EDIT_FILE],
  ["NotebookEdit", EDIT_FILE],
  ["write_file", EDIT_FILE],
  ["edit_file", EDIT_FILE],
  ["Bash", COMMAND],
  ["shell", COMMAND],
  ["Glob", SEARCH_FILES],
  ["Grep", SEARCH_FILES],
  ["LS", SEARCH_FILES],
  ["glob", SEARCH_FILES],
  ["grep", SEARCH_FILES],
  ["list_dir", SEARCH_FILES],
  ["WebFetch", WEB_PAGE],
  ["WebSearch", WEB_SEARCH],
]);

/// The phrase of one step: its tool's own, else "Used" and the name its chip shows.
function stepPhrase(tool: string, input: unknown, owners: ReadonlyMap<string, string>): StepPhrase {
  const known = STEP_PHRASES.get(tool);
  if (known) return known;
  const label = mcpToolLabel(callLabel(tool, input), owners);
  return { one: `Used ${label}`, many: (n) => `Used ${label} ${n} times` };
}

/// The one line a group of consecutive tool steps shows as in the conversation (conversational
/// chat spec section 5): the steps counted by phrase, in the order each phrase first comes, as
/// "Read 2 files · Ran a command". `owners`: MCP server name to the plugin it comes with.
export function stepsLine(
  items: { tool: string; input: unknown }[],
  owners: ReadonlyMap<string, string>,
): string {
  // A phrase's one-step words tell it apart: two tools that share a phrase share them.
  const counts = new Map<string, { phrase: StepPhrase; n: number }>();
  for (const item of items) {
    const phrase = stepPhrase(item.tool, item.input, owners);
    const seen = counts.get(phrase.one);
    if (seen) seen.n += 1;
    else counts.set(phrase.one, { phrase, n: 1 });
  }
  return [...counts.values()]
    .map(({ phrase, n }) => (n === 1 ? phrase.one : phrase.many(n)))
    .join(" · ");
}
```

**In `ui/src/screens/home/TopicRuns.tsx`, replace:**

```tsx
        dotName={dotName}
      />
```

**with:**

```tsx
        dotName={dotName}
        variant="conversation"
      />
```

- [ ] **Step 4: Run the task's tests**

Run: the command of Step 2.
Expected: PASS, 86 tests in 5 files.

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test --maxWorkers=2`, `bun run --cwd ui typecheck`.
Expected: 849 tests in 84 files pass; typecheck clean.

- [ ] **Step 6: Commit**

```bash
git add ui/src/lib/transcript.ts ui/src/components/Transcript.tsx ui/src/screens/home/TopicRuns.tsx ui/src/lib/transcript.test.ts ui/src/screens/RunScreen.test.tsx ui/src/screens/home/Document.test.tsx ui/src/screens/home/MemoryNote.test.tsx ui/src/screens/home/Conversation.test.tsx
git commit -m "feat(ui): the conversation shows a reply as plain text and tool steps as one line"
```

### Task 2: The conversation and its composer share one centred column

Spec section 5, "Reading column" and "Composer": messages, cards and composer sit in one column of at most 720px. The composer is a rounded box whose textarea grows with its text, with a round send button inside it.

**Files:**

- Modify: `ui/src/screens/home/Timeline.tsx`, `ui/src/screens/home/ReplyArea.tsx`, `ui/src/screens/home/Composer.tsx`
- Test: `ui/src/screens/home/Sending.test.tsx`

**Interfaces:**

- Consumes: nothing new.
- Produces: the Send button keeps the name "Send", and the textarea keeps the label "Message" (`#chat-message`), so later tests find them as before.

- [ ] **Step 1: Write the failing tests**

**In `ui/src/screens/home/Sending.test.tsx`, replace:**

```tsx
    expect(screen.getByRole("region", { name: "Conversation" })).toBeVisible();
  });
});
```

**with:**

```tsx
    expect(screen.getByRole("region", { name: "Conversation" })).toBeVisible();
  });
});

describe("the composer (conversational chat spec section 5)", () => {
  it("holds the text and a round Send button in one box under the conversation", async () => {
    home([chat("a", "10:00")]);
    renderApp(`/home/${dot.id}`);
    const found = await readyBox();
    const send = screen.getByRole("button", { name: "Send" });
    expect(send).toHaveAttribute("title", "Send");
    expect(send).toHaveClass("rounded-full");
    // The button sits in the box with the text, after it.
    const container = send.parentElement!;
    expect(container).toContainElement(found);
    expect(container).toHaveClass("rounded-2xl", "border");
    expect(found.compareDocumentPosition(send) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(found).toHaveAttribute("rows", "1");
    expect(found).toHaveClass("field-sizing-content", "max-h-40", "resize-none");
  });

  it("offers Send only for a message with text, and not while the box is locked", async () => {
    const working = chat("a", "10:00", { status: "running", ended_at: null });
    const runs = home([working]);
    const { user } = renderApp(`/home/${dot.id}`);
    await waitFor(() =>
      expect(box()).toHaveAttribute("placeholder", `${dot.name} is working on this…`),
    );
    const send = screen.getByRole("button", { name: "Send" });
    expect(box()).toBeDisabled();
    expect(send).toBeDisabled();
    expect(send.parentElement).toHaveClass("opacity-60");

    runs[0] = { ...working, status: "succeeded", ended_at: sample.run.ended_at };
    act(() => backend.emit("runtime", { type: "run_updated", run: runs[0] }));
    const found = await readyBox();
    expect(send).toBeDisabled();
    await user.type(found, "   ");
    expect(send).toBeDisabled();
    await user.type(found, "Next?");
    expect(send).toBeEnabled();
    expect(send.parentElement).not.toHaveClass("opacity-60");
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test --maxWorkers=2 src/screens/home/Sending.test.tsx`
Expected: FAIL, 2 failed and 21 passed (23): the two composer tests (no round button, no `field-sizing-content`).

- [ ] **Step 3: Centre the column and round the composer**

**In `ui/src/screens/home/Composer.tsx`, replace:**

```tsx
import { Send } from "lucide-react";
import { useEffect, useRef, useState } from "react";
```

**with:**

```tsx
import { cn } from "cn";
import { ArrowUp } from "lucide-react";
import { useEffect, useRef, useState } from "react";
```

**In `ui/src/screens/home/Composer.tsx`, replace:**

```tsx
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { useComposerReport } from "@/lib/use-activity";
```

**with:**

```tsx
import { Button } from "@/components/ui/button";
import { useComposerReport } from "@/lib/use-activity";
```

**In `ui/src/screens/home/Composer.tsx`, replace:**

```tsx

/// Where a message is typed. Enter sends; Shift+Enter starts a new line. The text is cleared
/// once the send has succeeded; a refused message stays for another try.
export function Composer(props: {
```

**with:**

```tsx

/// Where a message is typed: one rounded box holding the text and, at its bottom right, the send
/// button (conversational chat spec section 5). The text grows with what is typed, up to a few
/// lines, then scrolls. Enter sends; Shift+Enter starts a new line. The text is cleared once the
/// send has succeeded; a refused message stays for another try.
export function Composer(props: {
```

**In `ui/src/screens/home/Composer.tsx`, replace:**

```tsx
  return (
    <div className="flex items-end gap-2 px-3 pb-3">
      <label htmlFor="chat-message" className="sr-only">
```

**with:**

```tsx
  return (
    // The box draws the focus ring for the text inside it, which has none of its own.
    <div
      className={cn(
        "flex items-end gap-2 rounded-2xl border bg-card p-2 shadow-xs focus-within:border-ring focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ring",
        props.disabled && "cursor-not-allowed opacity-60",
      )}
    >
      <label htmlFor="chat-message" className="sr-only">
```

**In `ui/src/screens/home/Composer.tsx`, replace:**

```tsx
      </label>
      <Textarea
        id="chat-message"
```

**with:**

```tsx
      </label>
      <textarea
        id="chat-message"
```

**In `ui/src/screens/home/Composer.tsx`, replace:**

```tsx
        ref={box}
        rows={2}
        placeholder={props.hint}
```

**with:**

```tsx
        ref={box}
        rows={1}
        placeholder={props.hint}
```

**In `ui/src/screens/home/Composer.tsx`, replace:**

```tsx
        }}
      />
      <Button disabled={props.disabled || text.trim() === ""} onClick={send}>
        <Send aria-hidden /> Send
      </Button>
```

**with:**

```tsx
        }}
        className="field-sizing-content max-h-40 min-w-0 flex-1 resize-none overflow-y-auto bg-transparent px-1.5 py-1 text-base outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed md:text-sm"
      />
      <Button
        size="icon-sm"
        className="shrink-0 rounded-full"
        aria-label="Send"
        title="Send"
        disabled={props.disabled || !hasText}
        onClick={send}
      >
        <ArrowUp aria-hidden />
      </Button>
```

**In `ui/src/screens/home/ReplyArea.tsx`, replace:**

```tsx

/// Below the conversation: why the target cannot be continued, the open question, the reply
/// target, and the composer. A message continues the target's last run, or starts a new chat
/// topic without one; an answer to the open question goes through `message_reply`.
```

**with:**

```tsx

/// Below the conversation, in its centred column: why the target cannot be continued, the open
/// question, the reply target, and the composer. A message continues the target's last run, or starts a new chat
/// topic without one; an answer to the open question goes through `message_reply`.
```

**In `ui/src/screens/home/ReplyArea.tsx`, replace:**

```tsx
  return (
    <div className="flex flex-col gap-2 border-t pt-2">
      {blocked && (
        <p role="status" className="px-3 text-destructive">
          org removed: {blocked}
```

**with:**

```tsx
  return (
    <div className="mx-auto flex w-full max-w-[720px] flex-col gap-2 px-4 pb-4">
      {blocked && (
        <p role="status" className="px-1 text-destructive">
          org removed: {blocked}
```

**In `ui/src/screens/home/ReplyArea.tsx`, replace:**

```tsx
      {closed && (
        <p className="flex items-center gap-2 px-3 text-muted-foreground">
          <span className="flex-1">
```

**with:**

```tsx
      {closed && (
        <p className="flex items-center gap-2 px-1 text-muted-foreground">
          <span className="flex-1">
```

**In `ui/src/screens/home/ReplyArea.tsx`, replace:**

```tsx
          aria-label="Open question"
          className="flex flex-wrap items-center gap-2 px-3"
        >
```

**with:**

```tsx
          aria-label="Open question"
          className="flex flex-wrap items-center gap-2 rounded-xl border bg-card px-3 py-2"
        >
```

**In `ui/src/screens/home/ReplyArea.tsx`, replace:**

```tsx
        aria-live="polite"
        className="flex items-center gap-1 px-3 text-xs text-muted-foreground"
      >
```

**with:**

```tsx
        aria-live="polite"
        className="flex items-center gap-1 px-1 text-xs text-muted-foreground"
      >
```

**In `ui/src/screens/home/Timeline.tsx`, replace:**

```tsx
        aria-label="Conversation"
        className="flex min-h-0 flex-1 flex-col gap-6 overflow-auto p-4"
      >
        {props.children}
        {proposals.isError && <ErrorNote error={proposals.error} />}
        {topics.map((topic) =>
          topic.kind === "chat" ? (
            <TopicRuns
              key={topic.rootId}
              runs={topic.runs}
              checkIns={checkIns}
              dotName={dotName}
              proposals={props.proposals}
            />
          ) : (
            <div key={topic.rootId} className="flex flex-col gap-4">
              <RunCard
                topic={topic}
                checkIns={checkIns}
```

**with:**

```tsx
        aria-label="Conversation"
        className="flex min-h-0 flex-1 flex-col overflow-auto"
      >
        {/* The scrolling area keeps the pane's width, so its scrollbar is at the pane's edge;
            what it shows is one centred column (conversational chat spec section 5). */}
        <div className="mx-auto flex w-full max-w-[720px] flex-col gap-6 px-4 py-4">
          {props.children}
          {proposals.isError && <ErrorNote error={proposals.error} />}
          {topics.map((topic) =>
            topic.kind === "chat" ? (
              <TopicRuns
                key={topic.rootId}
                runs={topic.runs}
                checkIns={checkIns}
```

**In `ui/src/screens/home/Timeline.tsx`, replace:**

```tsx
                dotName={dotName}
                onReply={() => props.onReply(topic)}
              />
              {/* An app the run sent for review waits under its card. */}
              <AppReviews runs={topic.runs.slice(0, 1)} dotName={dotName} />
              {topic.runs.length > 1 && (
                <TopicRuns runs={topic.runs.slice(1)} checkIns={checkIns} dotName={dotName} />
              )}
            </div>
          ),
        )}
      </section>
```

**with:**

```tsx
                dotName={dotName}
                proposals={props.proposals}
              />
            ) : (
              <div key={topic.rootId} className="flex flex-col gap-4">
                <RunCard
                  topic={topic}
                  checkIns={checkIns}
                  dotName={dotName}
                  onReply={() => props.onReply(topic)}
                />
                {/* An app the run sent for review waits under its card. */}
                <AppReviews runs={topic.runs.slice(0, 1)} dotName={dotName} />
                {topic.runs.length > 1 && (
                  <TopicRuns runs={topic.runs.slice(1)} checkIns={checkIns} dotName={dotName} />
                )}
              </div>
            ),
          )}
        </div>
      </section>
```

- [ ] **Step 4: Run the task's tests**

Run: the command of Step 2.
Expected: PASS, 23 tests.

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test --maxWorkers=2`, `bun run --cwd ui typecheck`.
Expected: 851 tests in 84 files pass; typecheck clean.

- [ ] **Step 6: Commit**

```bash
git add ui/src/screens/home/Timeline.tsx ui/src/screens/home/ReplyArea.tsx ui/src/screens/home/Composer.tsx ui/src/screens/home/Sending.test.tsx
git commit -m "feat(ui): the conversation and its composer share one centred column"
```

### Task 3: The dot's name opens its menu, and the Dots column starts with New dot

Spec section 5, "Header" and "Dots column": the header is slim, with the dot's name and ▾ opening the menu ⚙ opened (Edit, Memory…, Run now, Enabled), and the status mark next to the name. The Dots column starts with a "New dot" row.

**Files:**

- Modify: `ui/src/screens/home/DotHeader.tsx`, `ui/src/screens/home/DotMenu.tsx`, `ui/src/screens/home/DotPane.tsx`, `ui/src/screens/home/DotsColumn.tsx`
- Test: `ui/src/screens/home/HomeMenu.test.tsx`, `ui/src/screens/home/HomeColumn.test.tsx`, `ui/src/lib/query.test.tsx`, `ui/src/screens/DotOrgs.test.tsx`, `ui/src/screens/dot-editor/MemoryTab.test.tsx`

**Interfaces:**

- Produces:
  - `DotHeader` takes `dot: Dot` (not `name`) with `status` and `children`, and renders the `DotMenu` itself, so `DotPane` no longer renders it.
  - The menu's trigger is a button whose accessible name is the dot's name, inside the `<h1>`; there is no button named "Dot settings" (the name is its `title`).
  - The "New dot" link keeps its name and target.

- [ ] **Step 1: Write the failing tests**

**In `ui/src/lib/query.test.tsx`, replace:**

```tsx
    const { user } = renderApp(`/home/${dot.id}`);
    const settings = await screen.findByRole("button", { name: "Dot settings" });

```

**with:**

```tsx
    const { user } = renderApp(`/home/${dot.id}`);
    const settings = await screen.findByRole("button", { name: dot.name });

```

**In `ui/src/screens/DotOrgs.test.tsx`, replace:**

```tsx

    await user.click(screen.getByRole("button", { name: "Dot settings" }));
    expect(await screen.findByRole("menuitem", { name: "Run now" })).toHaveAttribute(
```

**with:**

```tsx

    await user.click(screen.getByRole("button", { name: release.name }));
    expect(await screen.findByRole("menuitem", { name: "Run now" })).toHaveAttribute(
```

**In `ui/src/screens/dot-editor/MemoryTab.test.tsx`, replace:**

```tsx

  it("opens from the dot's ⚙ menu", async () => {
    home([]);
```

**with:**

```tsx

  it("opens from the dot's menu", async () => {
    home([]);
```

**In `ui/src/screens/dot-editor/MemoryTab.test.tsx`, replace:**

```tsx
    const { user, router } = renderApp(`/home/${dot.id}`);
    await user.click(await screen.findByRole("button", { name: "Dot settings" }));
    await user.click(await screen.findByRole("menuitem", { name: "Memory…" }));
```

**with:**

```tsx
    const { user, router } = renderApp(`/home/${dot.id}`);
    await user.click(await screen.findByRole("button", { name: dot.name }));
    await user.click(await screen.findByRole("menuitem", { name: "Memory…" }));
```

**In `ui/src/screens/home/HomeColumn.test.tsx`, replace:**

```tsx
    expect(screen.getByRole("link", { name: "New dot" })).toHaveAttribute("href", "/dots/new");
  });
```

**with:**

```tsx
    expect(screen.getByRole("link", { name: "New dot" })).toHaveAttribute("href", "/dots/new");
  });

  it("starts with New dot, then the heading over the list", async () => {
    backend.on("dot_overview", () => [reader, shopper]);
    renderApp(`/home/${reader.dot.id}`);
    const view = within(await screen.findByRole("navigation", { name: "Dots" }));
    const create = view.getByRole("link", { name: "New dot" });
    expect(create).toHaveTextContent("New dot");
    expect(create).toHaveAttribute("href", "/dots/new");
    const heading = view.getByRole("heading", { name: "Dots" });
    const list = view.getByRole("list");
    const before = (a: Node, b: Node) =>
      (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
    expect(before(create, heading)).toBe(true);
    expect(before(heading, list)).toBe(true);
  });
```

**In `ui/src/screens/home/HomeColumn.test.tsx`, replace:**

```tsx

  it("shows the dot's name and 'off' in the header of a disabled dot", async () => {
    const off = row("paused", {
```

**with:**

```tsx

  it("shows the dot's name and its 'off' mark in the header of a disabled dot", async () => {
    const off = row("paused", {
```

**In `ui/src/screens/home/HomeColumn.test.tsx`, replace:**

```tsx
    )!;
    expect(within(header).getByText("off")).toBeVisible();
  });
```

**with:**

```tsx
    )!;
    expect(within(header).getByRole("img", { name: "off" })).toBeVisible();
    // The mark says it; the header has no status text of its own.
    expect(within(header).queryByText("off")).toBeNull();
  });
```

**In `ui/src/screens/home/HomeMenu.test.tsx`, replace:**

```tsx

/// Opens the header's ⚙ menu.
async function openMenu(user: ReturnType<typeof renderApp>["user"]) {
  await user.click(await screen.findByRole("button", { name: "Dot settings" }));
  return screen.findByRole("menu");
```

**with:**

```tsx

/// Opens the menu under the dot's name in the header.
async function openMenu(user: ReturnType<typeof renderApp>["user"]) {
  await user.click(await screen.findByRole("button", { name: dot.name }));
  return screen.findByRole("menu");
```

**In `ui/src/screens/home/HomeMenu.test.tsx`, replace:**

```tsx

describe("the dot's ⚙ menu", () => {
  it("opens the dot's editor", async () => {
```

**with:**

```tsx

describe("the dot's menu", () => {
  it("opens from the dot's name with Edit, Memory…, Run now and Enabled", async () => {
    home([]);
    const { user } = renderApp(`/home/${dot.id}`);
    const name = await screen.findByRole("button", { name: dot.name });
    // The name is the header's heading, and the menu's only way in: there is no ⚙.
    expect(screen.getByRole("heading", { level: 1, name: dot.name })).toContainElement(name);
    expect(name).toHaveAttribute("title", "Dot settings");
    expect(screen.queryByRole("button", { name: "Dot settings" })).toBeNull();
    await user.click(name);
    const menu = await screen.findByRole("menu");
    const items = [...menu.querySelectorAll('[role="menuitem"], [role="menuitemcheckbox"]')];
    expect(items.map((item) => item.textContent)).toEqual([
      "Edit",
      "Memory…",
      "Run now",
      "Enabled",
    ]);
  });

  it("opens from the keyboard", async () => {
    home([]);
    const { user } = renderApp(`/home/${dot.id}`);
    const name = await screen.findByRole("button", { name: dot.name });
    act(() => name.focus());
    await user.keyboard("{Enter}");
    expect(
      within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Edit" }),
    ).toBeVisible();
  });

  it("opens the dot's editor", async () => {
```

**In `ui/src/screens/home/HomeMenu.test.tsx`, replace:**

```tsx
    const header = screen.getByRole("heading", { level: 1, name: dot.name }).closest("header")!;
    expect(await within(header).findByText("off")).toBeVisible();
    menu = await openMenu(user);
```

**with:**

```tsx
    const header = screen.getByRole("heading", { level: 1, name: dot.name }).closest("header")!;
    expect(await within(header).findByRole("img", { name: "off" })).toBeVisible();
    menu = await openMenu(user);
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test --maxWorkers=2 src/lib/query.test.tsx src/screens/DotOrgs.test.tsx src/screens/dot-editor/MemoryTab.test.tsx src/screens/home/HomeColumn.test.tsx src/screens/home/HomeMenu.test.tsx`
Expected: FAIL, 15 failed and 32 passed (47): every test that opens the menu finds no button named by the dot's name, and the header still shows its status words.

- [ ] **Step 3: Move the menu to the name**

**In `ui/src/screens/home/DotHeader.tsx`, replace:**

```tsx

import { STATUS_LABEL, type DotStatus } from "@/lib/dot-status";
import { StatusMark } from "@/screens/home/StatusMark";
```

**with:**

```tsx

import type { Dot } from "@/api/types";
import type { DotStatus } from "@/lib/dot-status";
import { DotMenu } from "@/screens/home/DotMenu";
import { StatusMark } from "@/screens/home/StatusMark";
```

**In `ui/src/screens/home/DotHeader.tsx`, replace:**

```tsx

/// The top of a dot's conversation: its name and status, then the actions it is given.
export function DotHeader(props: { name: string; status: DotStatus; children?: ReactNode }) {
  return (
```

**with:**

```tsx

/// The top of a dot's conversation, slim and with no bar (conversational chat spec section 5):
/// its name, which opens its menu, and its status mark, then the actions it is given.
export function DotHeader(props: { dot: Dot; status: DotStatus; children?: ReactNode }) {
  return (
```

**In `ui/src/screens/home/DotHeader.tsx`, replace:**

```tsx
    // Wraps its actions onto a second line in a narrow window rather than scrolling sideways.
    <header className="flex flex-wrap items-center gap-x-2 gap-y-1 border-b px-4 py-2">
      <h1 className="min-w-0 truncate text-base font-semibold">{props.name}</h1>
      <StatusMark status={props.status} />
      <span className="flex-1 text-xs text-muted-foreground">{STATUS_LABEL[props.status]}</span>
      {props.children}
```

**with:**

```tsx
    // Wraps its actions onto a second line in a narrow window rather than scrolling sideways.
    <header className="flex flex-wrap items-center gap-x-2 gap-y-1 px-4 py-2">
      <h1 className="flex min-w-0">
        <DotMenu dot={props.dot} />
      </h1>
      <StatusMark status={props.status} />
      <span className="flex-1" />
      {props.children}
```

**In `ui/src/screens/home/DotMenu.tsx`, replace:**

```tsx
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Check, Settings } from "lucide-react";
import { DropdownMenu } from "radix-ui";
```

**with:**

```tsx
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Check, ChevronDown } from "lucide-react";
import { DropdownMenu } from "radix-ui";
```

**In `ui/src/screens/home/DotMenu.tsx`, replace:**

```tsx

/// The header's ⚙: what the Dots list offered for a dot. Edit opens its editor, Memory… its
/// editor's Memory tab, Run now starts a run (it shows up in the conversation as a card),
/// Enabled switches it on or off.
export function DotMenu({ dot }: { dot: Dot }) {
```

**with:**

```tsx

/// The dot's name in the header, which opens what the Dots list offered for a dot. Edit opens
/// its editor, Memory… its editor's Memory tab, Run now starts a run (it shows up in the
/// conversation as a card), Enabled switches it on or off.
export function DotMenu({ dot }: { dot: Dot }) {
```

**In `ui/src/screens/home/DotMenu.tsx`, replace:**

```tsx
      <DropdownMenu.Trigger asChild>
        <Button size="icon-sm" variant="ghost" aria-label="Dot settings" title="Dot settings">
          <Settings aria-hidden />
        </Button>
```

**with:**

```tsx
      <DropdownMenu.Trigger asChild>
        {/* Named by the dot's name; the tooltip says what it opens. */}
        <Button
          variant="ghost"
          title="Dot settings"
          className="min-w-0 max-w-full text-base font-semibold"
        >
          <span className="truncate">{dot.name}</span>
          <ChevronDown aria-hidden className="text-muted-foreground" />
        </Button>
```

**In `ui/src/screens/home/DotMenu.tsx`, replace:**

```tsx
        <DropdownMenu.Content
          align="end"
          className="z-50 w-48 rounded-md border bg-popover p-1 text-sm text-popover-foreground shadow-md"
```

**with:**

```tsx
        <DropdownMenu.Content
          align="start"
          className="z-50 w-48 rounded-md border bg-popover p-1 text-sm text-popover-foreground shadow-md"
```

**In `ui/src/screens/home/DotPane.tsx`, replace:**

```tsx
import { DotHeader } from "@/screens/home/DotHeader";
import { DotMenu } from "@/screens/home/DotMenu";
import { HistoryMenu } from "@/screens/home/HistoryMenu";
```

**with:**

```tsx
import { DotHeader } from "@/screens/home/DotHeader";
import { HistoryMenu } from "@/screens/home/HistoryMenu";
```

**In `ui/src/screens/home/DotPane.tsx`, replace:**

```tsx
    <div className="flex min-w-0 flex-1 flex-col">
      <DotHeader name={dot.name} status={dotStatus(row, allMessages, held)}>
        <DotMenu dot={dot} />
        <HistoryMenu dotId={dot.id} topics={view.older} />
```

**with:**

```tsx
    <div className="flex min-w-0 flex-1 flex-col">
      <DotHeader dot={dot} status={dotStatus(row, allMessages, held)}>
        <HistoryMenu dotId={dot.id} topics={view.older} />
```

**In `ui/src/screens/home/DotPane.tsx`, replace:**

```tsx
      {rootRunId !== undefined && (
        <p className="border-b px-4 py-1.5 text-xs text-muted-foreground">
          Older topic ·{" "}
```

**with:**

```tsx
      {rootRunId !== undefined && (
        <p className="mx-auto w-full max-w-[720px] px-4 py-1.5 text-xs text-muted-foreground">
          Older topic ·{" "}
```

**In `ui/src/screens/home/DotsColumn.tsx`, replace:**

```tsx
    <nav aria-label="Dots" className="flex w-40 shrink-0 flex-col border-r bg-sidebar sm:w-64">
      <div className="flex items-center gap-2 px-3 py-2">
        <h2 className="flex-1 font-semibold">Dots</h2>
        <Button asChild size="icon-sm" variant="ghost">
          <Link to="/dots/new" aria-label="New dot" title="New dot">
            <Plus aria-hidden />
          </Link>
```

**with:**

```tsx
    <nav aria-label="Dots" className="flex w-40 shrink-0 flex-col border-r bg-sidebar sm:w-64">
      {/* New dot first, then the list under a quiet heading, as a chat app's column. */}
      <div className="px-2 pt-2">
        <Button asChild variant="ghost" className="w-full justify-start">
          <Link to="/dots/new">
            <Plus aria-hidden />
            New dot
          </Link>
```

**In `ui/src/screens/home/DotsColumn.tsx`, replace:**

```tsx
      </div>
      <ul className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-auto px-2 pb-2" onKeyDown={onKeyDown}>
```

**with:**

```tsx
      </div>
      <h2 className="px-4 pt-3 pb-1 text-xs font-medium text-muted-foreground">Dots</h2>
      <ul className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-auto px-2 pb-2" onKeyDown={onKeyDown}>
```

- [ ] **Step 4: Run the task's tests**

Run: the command of Step 2.
Expected: PASS, 47 tests in 5 files.

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test --maxWorkers=2`, `bun run --cwd ui typecheck`.
Expected: 854 tests in 84 files pass; typecheck clean.

- [ ] **Step 6: Commit**

```bash
git add ui/src/screens/home/DotHeader.tsx ui/src/screens/home/DotMenu.tsx ui/src/screens/home/DotPane.tsx ui/src/screens/home/DotsColumn.tsx ui/src/screens/home/HomeMenu.test.tsx ui/src/screens/home/HomeColumn.test.tsx ui/src/lib/query.test.tsx ui/src/screens/DotOrgs.test.tsx ui/src/screens/dot-editor/MemoryTab.test.tsx
git commit -m "feat(ui): the dot's name opens its menu, and the dots column starts with New dot"
```

### Task 4: An empty conversation greets with the dot's name

Spec section 5, "Empty conversation": the dot's name and "Ask <dot> something" centred in the column, with the composer under them, as Claude's new-chat screen.

**Files:**

- Modify: `ui/src/screens/home/DotPane.tsx`
- Test: `ui/src/screens/home/Conversation.test.tsx`, `ui/src/screens/home/Sending.test.tsx`

**Interfaces:**

- Consumes: Task 3's `DotHeader`.
- Produces: an empty current conversation has no region named "Conversation" until its first run loads; the `ReplyArea` keeps its place among `DotPane`'s children, so a send in progress is not mounted again.

- [ ] **Step 1: Write the failing tests**

**In `ui/src/screens/home/Conversation.test.tsx`, replace:**

```tsx

  it("invites the first message to a dot that never ran", async () => {
    home([]);
```

**with:**

```tsx

  it("greets with the dot's name over the composer when the conversation is empty", async () => {
    home([]);
```

**In `ui/src/screens/home/Conversation.test.tsx`, replace:**

```tsx
    renderApp(`/home/${dot.id}`);
    expect(await screen.findByText(`Ask ${dot.name} something.`, { exact: false })).toBeVisible();
  });
```

**with:**

```tsx
    renderApp(`/home/${dot.id}`);
    const name = await screen.findByRole("heading", { level: 2, name: dot.name });
    expect(name).toHaveClass("text-2xl", "font-semibold");
    const invite = screen.getByText(`Ask ${dot.name} something`);
    expect(invite).toHaveClass("text-muted-foreground");
    // The old line is gone, and there is no conversation to scroll.
    expect(screen.queryByText(/It answers with its own instructions/)).toBeNull();
    expect(screen.queryByRole("region", { name: "Conversation" })).toBeNull();
    // The composer is right under the greeting.
    const box = screen.getByLabelText("Message");
    expect(invite.compareDocumentPosition(box) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.getByRole("button", { name: "Send" })).toBeVisible();
  });

  it("does not greet when the dot's runs could not be loaded", async () => {
    home([]);
    backend.on("list_runs", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByRole("alert")).toHaveTextContent("database is locked");
    expect(screen.queryByText(`Ask ${dot.name} something`)).toBeNull();
  });

  it("turns the greeting into the conversation with the first message", async () => {
    home([]);
    const { user } = renderApp(`/home/${dot.id}`);
    await screen.findByText(`Ask ${dot.name} something`);
    const box = screen.getByLabelText("Message");
    await waitFor(() => expect(box).toBeEnabled());
    await user.type(box, "Hello there{Enter}");
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByText("Hello there")).toBeVisible();
    expect(screen.queryByText(`Ask ${dot.name} something`)).toBeNull();
    expect(screen.queryByRole("heading", { level: 2, name: dot.name })).toBeNull();
  });
```

**In `ui/src/screens/home/Sending.test.tsx`, replace:**

```tsx
    );
    const view = within(screen.getByRole("region", { name: "Conversation" }));
    expect(await view.findByText("Why did the build fail?")).toBeVisible();
```

**with:**

```tsx
    );
    // The empty conversation's greeting gives way to the conversation once the run is loaded.
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByText("Why did the build fail?")).toBeVisible();
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test --maxWorkers=2 src/screens/home/Conversation.test.tsx src/screens/home/Sending.test.tsx`
Expected: FAIL, 2 failed and 43 passed (45): "greets with the dot's name over the composer when the conversation is empty" and "turns the greeting into the conversation with the first message". "does not greet when the dot's runs could not be loaded" passes already: it guards the error path.

- [ ] **Step 3: Greet in place of the timeline**

**In `ui/src/screens/home/DotPane.tsx`, replace:**

```tsx
  const [pick, setPick] = useState<ReplyPick>({ kind: "default" });
  if (gone) return <Navigate to={`/home/${dot.id}`} replace />;
```

**with:**

```tsx
  const [pick, setPick] = useState<ReplyPick>({ kind: "default" });
  // The current conversation, loaded and with nothing in it, greets like a new chat
  // (conversational chat spec section 5). A failed load shows its errors instead.
  const greeting =
    rootRunId === undefined && !view.pending && view.list.isSuccess && view.shown.length === 0;
  if (gone) return <Navigate to={`/home/${dot.id}`} replace />;
```

**In `ui/src/screens/home/DotPane.tsx`, replace:**

```tsx
      )}
      <Timeline
        topics={view.pending ? [] : view.shown}
        checkIns={view.checkIns}
        dotName={dot.name}
        proposals={proposals}
        onReply={(topic) => setPick({ kind: "topic", rootId: topic.rootId })}
      >
        <LoadErrors queries={[view.list, ...view.threads]} />
        {!view.pending && view.shown.length === 0 && view.list.isSuccess && (
          <p className="text-muted-foreground">
            Ask {dot.name} something. It answers with its own instructions, folder and
            permissions.
          </p>
        )}
      </Timeline>
      {!view.pending && (
```

**with:**

```tsx
      )}
      {greeting ? (
        // Above the composer and pushed down by as much space as the spacer under it, so that
        // the greeting and the composer sit together in the middle of the pane.
        <div className="mx-auto flex w-full max-w-[720px] flex-1 flex-col items-center justify-end gap-1 px-4 pb-6 text-center">
          <h2 className="text-2xl font-semibold">{dot.name}</h2>
          <p className="text-muted-foreground">Ask {dot.name} something</p>
        </div>
      ) : (
        <Timeline
          topics={view.pending ? [] : view.shown}
          checkIns={view.checkIns}
          dotName={dot.name}
          proposals={proposals}
          onReply={(topic) => setPick({ kind: "topic", rootId: topic.rootId })}
        >
          <LoadErrors queries={[view.list, ...view.threads]} />
        </Timeline>
      )}
      {/* The reply area keeps its place in the tree when the greeting gives way to the
          conversation, so a send on its way is not cut off. */}
      {!view.pending && (
```

**In `ui/src/screens/home/DotPane.tsx`, replace:**

```tsx
      )}
    </div>
```

**with:**

```tsx
      )}
      {greeting && <div aria-hidden className="flex-1" />}
    </div>
```

- [ ] **Step 4: Run the task's tests**

Run: the command of Step 2.
Expected: PASS, 45 tests in 2 files.

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test --maxWorkers=2`, `bun run --cwd ui typecheck`.
Expected: 856 tests in 84 files pass; typecheck clean.

- [ ] **Step 6: Commit**

```bash
git add ui/src/screens/home/DotPane.tsx ui/src/screens/home/Conversation.test.tsx ui/src/screens/home/Sending.test.tsx
git commit -m "feat(ui): an empty conversation greets with the dot's name"
```

### Task 5: Run cards are rounded cards, and a proposal waits while its topic works

Spec section 5, "Run cards …": a run topic's card becomes a rounded light-bordered card. Spec section 9, "Known gap": the instruction card's Approve and Reject are disabled while the topic works.

**Files:**

- Modify: `ui/src/screens/home/RunCard.tsx`, `ui/src/components/InstructionsCard.tsx`, `ui/src/screens/home/TopicRuns.tsx`
- Test: `ui/src/screens/home/Instructions.test.tsx`, `ui/src/screens/home/Conversation.test.tsx`

**Interfaces:**

- Consumes: Task 1's `TopicRuns` (it passes `variant="conversation"`).
- Produces: `InstructionsCard`'s prop `busy?: boolean` (default false). `TopicRuns` passes `runs.some(isActive)`; the Inbox passes nothing.

- [ ] **Step 1: Write the failing tests**

**In `ui/src/screens/home/Conversation.test.tsx`, replace:**

```tsx
    await listening();
    expect(within(shown).getByText("Succeeded")).toBeVisible();
```

**with:**

```tsx
    await listening();
    // A rounded light-bordered card, not a rule on its left.
    expect(shown).toHaveClass("rounded-xl", "border", "bg-card");
    expect(within(shown).getByText("Succeeded")).toBeVisible();
```

**In `ui/src/screens/home/Instructions.test.tsx`, replace:**

```tsx
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
```

**with:**

```tsx
import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
```

**In `ui/src/screens/home/Instructions.test.tsx`, replace:**

```tsx
import { backend } from "@/test/backend";
import { chat, dot, home } from "@/test/home";
```

**with:**

```tsx
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { chat, dot, home } from "@/test/home";
```

**In `ui/src/screens/home/Instructions.test.tsx`, replace:**

```tsx

  it("lists the waiting ones in the Inbox with their dot and conversation", async () => {
```

**with:**

```tsx

  it("waits to be decided while the dot still works on its topic", async () => {
    const working = { ...a, status: "running" as const, ended_at: null };
    const runs = home([working]);
    backend.on("instruction_proposals_list", () => [proposal()]);
    renderApp(`/home/${dot.id}`);
    const shown = await card();
    const wait = `${dot.name} is still working on this topic`;
    for (const name of ["Approve", "Reject"]) {
      const button = within(shown).getByRole("button", { name });
      expect(button).toBeDisabled();
      expect(button).toHaveAttribute("title", wait);
    }

    runs[0] = { ...working, status: "succeeded", ended_at: sample.run.ended_at };
    act(() => backend.emit("runtime", { type: "run_updated", run: runs[0] }));
    await waitFor(() =>
      expect(within(shown).getByRole("button", { name: "Approve" })).toBeEnabled(),
    );
    expect(within(shown).getByRole("button", { name: "Reject" })).toBeEnabled();
    expect(within(shown).getByRole("button", { name: "Approve" })).not.toHaveAttribute("title");
  });

  it("lists the waiting ones in the Inbox with their dot and conversation", async () => {
```

**In `ui/src/screens/home/Instructions.test.tsx`, replace:**

```tsx
      `/dots/${dot.id}`,
    );
  });
});
```

**with:**

```tsx
      `/dots/${dot.id}`,
    );
    // The Inbox does not hold the topic's runs: its card is never made to wait.
    for (const name of ["Approve", "Reject"]) {
      expect(within(group).getByRole("button", { name })).toBeEnabled();
    }
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test --maxWorkers=2 src/screens/home/Instructions.test.tsx src/screens/home/Conversation.test.tsx`
Expected: FAIL, 2 failed and 26 passed (28): "waits to be decided while the dot still works on its topic" and the card class in "shows a closed run card without loading its events".

- [ ] **Step 3: Round the run card and hold the proposal**

**In `ui/src/components/InstructionsCard.tsx`, replace:**

```tsx
/// person approves; the dot reads the answer as its chat's next message.
export function InstructionsCard(props: { proposal: InstructionProposal; dotName: string }) {
  const { proposal, dotName } = props;
  const qc = useQueryClient();
```

**with:**

```tsx
/// person approves; the dot reads the answer as its chat's next message.
export function InstructionsCard(props: {
  proposal: InstructionProposal;
  dotName: string;
  /// The dot still works on the proposal's topic. The core refuses a decision until the topic's
  /// runs have ended, so Approve and Reject wait too.
  busy?: boolean;
}) {
  const { proposal, dotName, busy = false } = props;
  const qc = useQueryClient();
```

**In `ui/src/components/InstructionsCard.tsx`, replace:**

```tsx
  const decided = decide.isPending || decide.isSuccess;
  const lines = lineDiff(proposal.base, proposal.instructions);
```

**with:**

```tsx
  const decided = decide.isPending || decide.isSuccess;
  const wait = busy ? `${dotName} is still working on this topic` : undefined;
  const lines = lineDiff(proposal.base, proposal.instructions);
```

**In `ui/src/components/InstructionsCard.tsx`, replace:**

```tsx
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" disabled={decided} onClick={onSingleClick(() => decide.mutate(true))}>
          Approve
```

**with:**

```tsx
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          disabled={decided || busy}
          title={wait}
          onClick={onSingleClick(() => decide.mutate(true))}
        >
          Approve
```

**In `ui/src/components/InstructionsCard.tsx`, replace:**

```tsx
          variant="outline"
          disabled={decided}
          onClick={onSingleClick(() => decide.mutate(false))}
```

**with:**

```tsx
          variant="outline"
          disabled={decided || busy}
          title={wait}
          onClick={onSingleClick(() => decide.mutate(false))}
```

**In `ui/src/screens/home/RunCard.tsx`, replace:**

```tsx
/// waits for an approval or has an open question the card is open and shows them; closed, it
/// loads nothing (chat-first home spec, section 4.2).
export function RunCard(props: {
```

**with:**

```tsx
/// waits for an approval or has an open question the card is open and shows them; closed, it
/// loads nothing (chat-first home spec, section 4.2). It is a rounded light-bordered card, as
/// the conversation's other cards are (conversational chat spec section 5).
export function RunCard(props: {
```

**In `ui/src/screens/home/RunCard.tsx`, replace:**

```tsx
      aria-label={topicTitle(topic)}
      className="flex flex-col gap-2 border-l-2 border-border py-1 pl-3"
    >
```

**with:**

```tsx
      aria-label={topicTitle(topic)}
      className="flex flex-col gap-2 rounded-xl border bg-card px-4 py-3"
    >
```

**In `ui/src/screens/home/TopicRuns.tsx`, replace:**

```tsx
import { Failure, Transcript } from "@/components/Transcript";
import { eventsQuery, keys } from "@/lib/query";
```

**with:**

```tsx
import { Failure, Transcript } from "@/components/Transcript";
import { isActive } from "@/lib/home-topics";
import { eventsQuery, keys } from "@/lib/query";
```

**In `ui/src/screens/home/TopicRuns.tsx`, replace:**

```tsx
  );
  return (
```

**with:**

```tsx
  );
  // A proposal can't be decided while the dot still works on its topic.
  const busy = runs.some(isActive);
  return (
```

**In `ui/src/screens/home/TopicRuns.tsx`, replace:**

```tsx
      {proposals.map((proposal) => (
        <InstructionsCard key={proposal.id} proposal={proposal} dotName={dotName} />
      ))}
```

**with:**

```tsx
      {proposals.map((proposal) => (
        <InstructionsCard key={proposal.id} proposal={proposal} dotName={dotName} busy={busy} />
      ))}
```

- [ ] **Step 4: Run the task's tests**

Run: the command of Step 2.
Expected: PASS, 28 tests in 2 files.

- [ ] **Step 5: Run the checks**

Run: `DEBUG_PRINT_LIMIT=0 bun run --cwd ui test --maxWorkers=2`, `bun run --cwd ui typecheck`.
Expected: 857 tests in 84 files pass; typecheck clean.

- [ ] **Step 6: Commit**

```bash
git add ui/src/screens/home/RunCard.tsx ui/src/components/InstructionsCard.tsx ui/src/screens/home/TopicRuns.tsx ui/src/screens/home/Instructions.test.tsx ui/src/screens/home/Conversation.test.tsx
git commit -m "feat(ui): run cards are rounded cards, and a proposal waits while its topic works"
```

### Task 6: The spec says what plan B built

Spec section 9 gains "Plan B (section 5)", the table "Deviations" above as built, and the known gap is marked closed.

**Files:**

- Modify: `docs/superpowers/specs/2026-10-27-conversational-chat-design.md`

- [ ] **Step 1: Write section 9's plan B list**

**In `docs/superpowers/specs/2026-10-27-conversational-chat-design.md`, replace:**

```markdown
- **The card's diff** is a line diff of the two texts (`lib/line-diff.ts`), not `DiffView`, which shows git patches.
- **Known gap:** the card's Approve and Reject are enabled while the proposing run is still finishing; deciding then is refused with the reply's "still working" words, and the card waits. Plan B's conversation layout disables them while the topic works.
```

**with:**

```markdown
- **The card's diff** is a line diff of the two texts (`lib/line-diff.ts`), not `DiffView`, which shows git patches.
- **Known gap, closed by plan B:** the card's Approve and Reject were enabled while the proposing run was still finishing, and deciding then was refused with the reply's "still working" words. The conversation now disables them while the topic works (the `busy` card below). The Inbox's card stays enabled, and the core still refuses a decision there while the topic works.

Plan B (section 5):

- **Transcript's look** comes from a `variant` prop, `"run"` by default: the run screen passes nothing and keeps its look; the conversation (`TopicRuns`) passes `"conversation"`. There, a run that a chat message started has no header line (its bubble says it), runs of other triggers keep theirs, and every run's section keeps its name; the dot's words have no "<dot> · time" line; a person's bubble is at most 75% of the column (the run screen keeps 80%) and breaks long words.
- **A group of tool steps** in the conversation is one `<details>` whose summary is a muted line: a chevron that turns when open, `stepsLine` (`lib/transcript.ts`), then " · N failed" in the destructive colour (a call parked for approval is not counted), " · waiting for approval" in the needs colour, and " · running…" while the run goes on and a step has no result. Opened, it shows the chips as before. The memory notes stay under the line, outside it. `stepsLine` counts the steps by phrase, in the order each phrase first comes, joined with " · ": "Read a file" (`Read`, `read_file`), "Edited a file" (`Write`, `Edit`, `MultiEdit`, `NotebookEdit`, `write_file`, `edit_file`), "Ran a command" (`Bash`, `shell`), "Searched the files" (`Glob`, `Grep`, `LS`, `glob`, `grep`, `list_dir`), "Read a web page" (`WebFetch`), "Searched the web" (`WebSearch`), and for any other tool "Used" with its chip's label (a plugin's server as its plugin, a local memory call as `Memory › remember`); "Read 2 files", "Searched the files 3 times", "Used X 2 times" for more than one.
- **The column** is 720px at most, centred, with 16px side padding: the conversation's content (its scrolling area keeps the pane's width, so the scrollbar is at the pane's edge), the reply area under it (no rule above it; the open question is a rounded light-bordered card) and the "Older topic" line.
- **The composer** is one rounded box with a borderless text area and a round send button (an up arrow, named "Send") at its bottom right. The text area starts one line high and grows with its text (`field-sizing-content`) up to 10rem, then scrolls. The box draws the focus ring while the focus is inside it, and is dimmed while locked.
- **The header** has no rule under it. The dot's name, a ghost button with a chevron in the `<h1>`, opens the menu that ⚙ opened (Edit, Memory…, Run now, Enabled), now aligned to the name's left edge; its tooltip is "Dot settings", its name is the dot's. The status mark follows the name and keeps its label and tooltip; the status text beside it is gone.
- **The Dots column** starts with a full-width "New dot" row, then a small muted "Dots" heading over the list.
- **The empty conversation** (the current one, loaded, with no topic) shows no conversation area: the dot's name as an `<h2>` and "Ask <dot> something" over the reply area, the three centred in the pane. A failed load shows its errors instead. The first message sent turns it into the conversation; the reply area stays mounted meanwhile.
- **A run card** is a rounded light-bordered card, contents unchanged.
- **The `busy` card:** `InstructionsCard` takes `busy`; `TopicRuns` passes whether a run of the topic is active, and then Approve and Reject are disabled with the tooltip "<dot> is still working on this topic". The Inbox passes nothing.
- **Not restyled:** `Failure` keeps the deep-teal spec's red rule on its left, as it is shared with the run screen. `ApprovalCard`, `InstructionsCard` and `AppReviewCard` were already rounded bordered cards, and a browser call already a rounded bordered line. Check-ins stay rounded soft cards filled with the muted colour, without a border.
```

- [ ] **Step 2: Check it**

Run: `git diff --check`, and read the new lines against the code of Tasks 1 to 5.
Expected: no whitespace error; every claim matches the code.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-10-27-conversational-chat-design.md
git commit -m "docs: the spec says what plan B built"
```
