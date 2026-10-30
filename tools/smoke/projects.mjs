// Projects' screen, walked in its sandboxed frame as a person would (app library spec section 8):
// a project, a section, a task added inline, its side pane, its spec in Markdown, a subtask,
// its Activity and an Undo, with Dots' theme on the page. Prints one line per check and exits 1
// when any failed, 2 when the frame is missing or does not answer, 0 otherwise.
//
// Needs Node 22 or later (the global WebSocket). Start a debug Dots with
// WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS holding `--remote-debugging-port=<port>` plus the
// release `browser_args(false)` (apps/dots-app/src/app_frame.rs), open `apps/projects`, then:
//
//   node tools/smoke/projects.mjs <port>                     the walk
//   node tools/smoke/projects.mjs <port> open-run "<task>"   opens the task, assigned to a dot,
//                                                            from Assigned to dots, and clicks
//                                                            its first "Open the run"
const port = Number(process.argv[2]);
const [mode, task] = [process.argv[3] ?? "walk", process.argv[4] ?? ""];
const ANSWER_MS = 30000;

const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const frame = targets.find((t) => t.type === "iframe" && t.url.includes("/app/projects/"));
if (!frame) {
  console.log("no Projects frame: open apps/projects first");
  process.exit(2);
}
const ws = new WebSocket(frame.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));

/// Runs `expression` (an async function's body) in the frame and gives what it returns.
function run(expression) {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve({ error: `no answer in ${ANSWER_MS / 1000} s` }), ANSWER_MS);
    ws.addEventListener("message", function on(m) {
      const d = JSON.parse(m.data);
      if (d.id !== 1) return;
      clearTimeout(timer);
      ws.removeEventListener("message", on);
      resolve(d.result?.result?.value ?? { error: d.result?.exceptionDetails?.exception?.description });
    });
    ws.send(
      JSON.stringify({
        id: 1,
        method: "Runtime.evaluate",
        params: { expression: `(async () => { ${expression} })()`, awaitPromise: true, returnByValue: true },
      }),
    );
  });
}

// The walk, in the page: each check is [name, passed, detail].
const walk = `
  const checks = [];
  const check = (name, ok, detail = "") => checks.push([name, Boolean(ok), String(detail)]);
  const $ = (sel) => document.querySelector(sel);
  const until = async (find, ms = 5000) => {
    for (let t = 0; t < ms; t += 100) {
      const got = find();
      if (got) return got;
      await new Promise((r) => setTimeout(r, 100));
    }
    return null;
  };
  const enter = (sel, text) => {
    const input = $(sel);
    input.value = text;
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  };
  const button = (text) => [...document.querySelectorAll("button")].find((b) => b.textContent === text);
  const name = "Smoke " + Math.floor(performance.now());

  check("theme", getComputedStyle(document.documentElement).getPropertyValue("--dots-primary"),
    document.documentElement.dataset.theme);
  check("left column", await until(() => button("My tasks") && button("Assigned to dots")));
  enter('[aria-label="New project"]', name);
  check("new project opens", await until(() => $("h1")?.textContent === name), $("h1")?.textContent);
  enter('[aria-label="New section"]', "To do");
  check("section added", await until(() => $('[aria-label="Add a task to To do"]')));
  enter('[aria-label="Add a task to To do"]', "Write the spec");
  const row = await until(() => $('[role="row"][aria-label="Write the spec"]'));
  check("task added inline", row);
  check("the cursor stays in the add row",
    await until(() => document.activeElement?.getAttribute("aria-label") === "Add a task to To do"));
  row?.querySelector(".title").click();
  const pane = await until(() => $('aside[aria-label="Task"]'));
  check("side pane", pane);
  button("Edit")?.click();
  const box = await until(() => $('textarea[aria-label="Spec"]'));
  box.value = "## Goal\\nShip **it**\\n\\n- [x] one\\n\\n<img src=x onerror=alert(1)>";
  button("Save").click();
  check("spec as Markdown", await until(() => $(".markdown h5") && $(".markdown strong")),
    $(".markdown")?.innerHTML);
  check("spec HTML stays text", !$(".markdown img"));
  enter('[aria-label="New subtask"]', "Tests");
  check("subtask added", await until(() => [...document.querySelectorAll(".subtasks li")]
    .some((li) => li.textContent.includes("Tests"))));
  const items = () => [...document.querySelectorAll('ol[aria-label="Activity"] li')];
  check("activity rows", await until(() => items().length >= 3), items().map((li) => li.textContent).join(" | "));
  const spec = items().find((li) => li.textContent.includes("changed the spec"));
  [...(spec?.querySelectorAll("button") ?? [])].find((b) => b.textContent === "Undo")?.click();
  check("undo", await until(() => items().some((li) => li.textContent.includes("(undone)"))),
    items().map((li) => li.textContent).join(" | "));
  check("undo put the spec back", await until(() => $(".markdown")?.textContent.includes("No spec yet")));
  return checks;
`;

// Opens a run from the task's Activity: Dots then shows the dot's conversation.
const openRun = `
  const checks = [];
  const check = (name, ok, detail = "") => checks.push([name, Boolean(ok), String(detail)]);
  const until = async (find, ms = 5000) => {
    for (let t = 0; t < ms; t += 100) {
      const got = find();
      if (got) return got;
      await new Promise((r) => setTimeout(r, 100));
    }
    return null;
  };
  const button = (text, root = document) =>
    [...root.querySelectorAll("button")].find((b) => b.textContent === text);
  button("Assigned to dots").click();
  const title = await until(() => button(${JSON.stringify(task)}));
  check("the task is assigned to a dot", title);
  title?.click();
  const open = await until(() => button("Open the run"));
  check("its Activity links the run", open);
  open?.click();
  return checks;
`;

const result = await run(mode === "open-run" ? openRun : walk);
ws.close();
if (!Array.isArray(result)) {
  console.log("the walk did not finish:", result?.error ?? result);
  process.exit(2);
}
let failed = false;
for (const [name, ok, detail] of result) {
  if (!ok) failed = true;
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${detail ? `: ${detail.slice(0, 160)}` : ""}`);
}
setTimeout(() => process.exit(failed ? 1 : 0), 100);
