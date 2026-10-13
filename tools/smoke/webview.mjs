// Drives the Dots window from a terminal, for smoke runs without a person at the screen.
// It talks to WebView2's remote-debugging port, which only exists when the app is started
// with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>.
//
//   node tools/smoke/webview.mjs <port> eval "<async JS expression>"   print its JSON result
//   node tools/smoke/webview.mjs <port> text                           print the visible text
//   node tools/smoke/webview.mjs <port> goto inbox                     open a route (no leading
//                                                                    slash: Git Bash would turn
//                                                                    "/inbox" into a Windows path)
//   node tools/smoke/webview.mjs <port> shot <file.png>                save a screenshot
//   node tools/smoke/webview.mjs <port> click "<name>" [n]             click a button, tab,
//                                                                    switch or link
//   node tools/smoke/webview.mjs <port> dblclick "<name>" [n]          double-click it
//   node tools/smoke/webview.mjs <port> fill "<name>" "<text>"         replace what a text
//                                                                    field holds
//   node tools/smoke/webview.mjs <port> wait "<text>" [seconds]        wait until the visible
//                                                                    text has <text> (15 s)
//
// In `eval`, `invoke` is the Tauri bridge: invoke("status"), invoke("run_now", {dotId}).
// After invoke("restart_app") the page closes before it answers; the script then says so and
// exits with code 1, which is the expected outcome of that one call.
//
// `click` and `dblclick` take a control's accessible name: its `aria-label`, or else its text,
// or else the text of its label. Only what is visible and enabled is clicked, with the mouse
// events a person's click makes, at the middle of the control. When several controls have the
// name, <n> picks one, counted from 1 in page order. `dblclick` waits between its two clicks
// as a person does, and prints what was under the pointer at the second one: the page may
// have changed by then.
//
// `fill` takes a field's `aria-label`, or else the text of its label, or else its placeholder.
// The text arrives as typed text does, so the page's own handlers see it. It cannot be empty.
//
// Every failure is one line on stderr. Exit codes: 0 done, 1 failed, 2 wrong arguments.
// Needs Node 22 or newer (global fetch and WebSocket).
import { writeFileSync } from "node:fs";

const USAGE =
  'usage: webview.mjs <port> eval "<expr>" | text | goto <route> | shot <file.png> | ' +
  'click "<name>" [n] | dblclick "<name>" [n] | fill "<name>" "<text>" | wait "<text>" [seconds]';
/// The actions, and how many arguments each needs.
const ACTIONS = { eval: 1, text: 0, goto: 0, shot: 1, click: 1, dblclick: 1, fill: 2, wait: 1 };
/// Between the two clicks of `dblclick`: inside the 500 ms Windows counts as a double-click.
const DOUBLE_CLICK_PAUSE_MS = 300;
const WAIT_SECONDS = 15;
const WAIT_EVERY_MS = 250;

const [port, action, arg, extra] = process.argv.slice(2);
const needed = Object.hasOwn(ACTIONS, action ?? "") ? ACTIONS[action] : NaN;
/// The optional number after the argument: <n> or <seconds>. `NaN` when it is not one.
const count =
  needed === 2 || extra === undefined ? undefined : /^[1-9]\d*$/.test(extra) ? Number(extra) : NaN;
const given = [arg, extra].filter(Boolean).length;
if (!/^\d+$/.test(port ?? "") || !(given >= needed) || Number.isNaN(count)) {
  console.error(USAGE);
  process.exit(2);
}

function fail(line) {
  console.error(line);
  process.exit(1);
}

const targets = await fetch(`http://127.0.0.1:${port}/json`)
  .then((response) => response.json())
  .catch(() =>
    fail(
      `nothing answers on debugging port ${port}: start the app with ` +
        `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=${port}`,
    ),
  );
const page = Array.isArray(targets) ? targets.find((t) => t.type === "page") : undefined;
if (!page) fail(`no page on debugging port ${port}`);
const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve) => {
  socket.onopen = resolve;
  socket.onerror = () => fail(`could not connect to the page on debugging port ${port}`);
});
// From here on an error of the socket is a page that went away, which `onclose` handles.
socket.onerror = null;

let lastId = 0;
const waiting = new Map();
socket.onmessage = (message) => {
  const reply = JSON.parse(message.data);
  waiting.get(reply.id)?.(reply);
  waiting.delete(reply.id);
};
// The page goes away when the app restarts or quits: answer what still waits, so the script
// ends instead of hanging.
socket.onclose = () => {
  for (const settle of waiting.values()) settle({ closed: true });
  waiting.clear();
};

/// One DevTools call and its result. Throws when the page closed first, and when the debugger
/// answered with an error instead of a result.
async function call(method, params = {}) {
  const reply = await new Promise((resolve) => {
    waiting.set(++lastId, resolve);
    socket.send(JSON.stringify({ id: lastId, method, params }));
  });
  if (reply.closed) throw new Error("the page closed before it answered");
  if (reply.error) {
    throw new Error(`the debugger refused ${method}: ${reply.error.message ?? "no reason given"}`);
  }
  return reply.result ?? {};
}

/// Evaluates an async expression in the page and returns its value (JSON-encoded there).
/// What the expression throws is JSON-encoded there too: a command the backend refuses
/// rejects with an object (`{ kind, message }`), of which the debugger would only say "Object".
async function evaluate(expression) {
  const result = await call("Runtime.evaluate", {
    expression: `(async () => {
      const invoke = window.__TAURI_INTERNALS__?.invoke;
      try {
        return JSON.stringify({ value: await (async () => (${expression}))() });
      } catch (thrown) {
        return JSON.stringify({ thrown: thrown instanceof Error ? String(thrown) : (thrown ?? null) });
      }
    })()`,
    awaitPromise: true,
    returnByValue: true,
  });
  // Only what the page could not run at all arrives here, such as a syntax error.
  const failure = result.exceptionDetails;
  if (failure) {
    const thrown = failure.exception?.value ?? failure.exception?.description ?? failure.text;
    throw new Error(`the page threw: ${JSON.stringify(thrown)}`);
  }
  const answer = JSON.parse(result.result?.value ?? "{}");
  if ("thrown" in answer) throw new Error(`the page threw: ${JSON.stringify(answer.thrown)}`);
  return answer.value;
}

/// A call of `fn`, which is defined here and runs in the page, with `args` as JSON.
const inPage = (fn, ...args) => `(${fn})(${args.map((a) => JSON.stringify(a ?? null)).join(", ")})`;

/// Runs in the page. Where to click the control named `name`: `{ x, y }`, the middle of it
/// after it was scrolled into view. `{ problem }` when no visible control has the name, when
/// several have it and `nth` picks none, when it is disabled, and when something covers it
/// (as the rest of the page is while a dialog is open).
function locate(name, nth) {
  const clean = (text) => (text ?? "").replace(/\s+/g, " ").trim();
  const nameOf = (el) =>
    clean(el.getAttribute("aria-label")) || clean(el.innerText) || clean(el.labels?.[0]?.innerText);
  const controls = [...document.querySelectorAll("button, a[href]")].filter(
    (el) => el.getClientRects().length > 0,
  );
  const named = controls.filter((el) => nameOf(el) === name);
  const quoted = JSON.stringify(name);
  if (named.length === 0) {
    const names = [...new Set(controls.map(nameOf).filter(Boolean))].map((n) => JSON.stringify(n));
    return { problem: `no control named ${quoted} (the page has: ${names.join(", ")})` };
  }
  if (nth === null && named.length > 1) {
    return { problem: `${named.length} controls are named ${quoted}: add a number, 1 to ${named.length}` };
  }
  const el = named[(nth ?? 1) - 1];
  if (!el) return { problem: `only ${named.length} controls are named ${quoted}` };
  if (el.disabled || el.getAttribute("aria-disabled") === "true") {
    return { problem: `${quoted} is disabled` };
  }
  el.scrollIntoView({ block: "center", inline: "center" });
  const box = el.getBoundingClientRect();
  const x = Math.round(box.left + box.width / 2);
  const y = Math.round(box.top + box.height / 2);
  const top = document.elementFromPoint(x, y);
  if (!top || !(el === top || el.contains(top))) {
    return { problem: `${quoted} is covered by another element` };
  }
  return { x, y };
}

/// Runs in the page. The control at a point, as `{ name, disabled }`, or `null` for none.
function controlAt(x, y) {
  const el = document.elementFromPoint(x, y)?.closest("button, a[href]");
  if (!el) return null;
  const clean = (text) => (text ?? "").replace(/\s+/g, " ").trim();
  return {
    name:
      clean(el.getAttribute("aria-label")) || clean(el.innerText) || clean(el.labels?.[0]?.innerText),
    disabled: el.disabled === true || el.getAttribute("aria-disabled") === "true",
  };
}

/// Runs in the page. Puts the focus in the text field named `name` with all of its text
/// selected, so that typed text replaces it. `{ problem }` when there is not exactly one such
/// field that is visible and can be typed in.
function focusField(name) {
  const clean = (text) => (text ?? "").replace(/\s+/g, " ").trim();
  const nameOf = (el) =>
    clean(el.getAttribute("aria-label")) || clean(el.labels?.[0]?.innerText) || clean(el.placeholder);
  const fields = [...document.querySelectorAll("input, textarea")].filter(
    (el) => el.getClientRects().length > 0,
  );
  const named = fields.filter((el) => nameOf(el) === name);
  const quoted = JSON.stringify(name);
  if (named.length !== 1) {
    const names = [...new Set(fields.map(nameOf).filter(Boolean))].map((n) => JSON.stringify(n));
    return { problem: `${named.length} fields are named ${quoted} (the page has: ${names.join(", ")})` };
  }
  const [el] = named;
  if (el.disabled || el.readOnly) return { problem: `${quoted} cannot be typed in` };
  el.scrollIntoView({ block: "center", inline: "center" });
  el.focus();
  el.select();
  return {};
}

/// One press and release of the left button. `clickCount` 2 makes it the second click of a
/// double-click: the page's click event then has `detail` 2.
async function press(x, y, clickCount) {
  const at = { x, y, button: "left", clickCount };
  await call("Input.dispatchMouseEvent", { ...at, type: "mousePressed", buttons: 1 });
  await call("Input.dispatchMouseEvent", { ...at, type: "mouseReleased" });
}

const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

let exitCode = 0;
try {
  if (action === "eval") {
    console.log(JSON.stringify(await evaluate(arg), null, 2));
  } else if (action === "text") {
    console.log(await evaluate("document.body.innerText"));
  } else if (action === "goto") {
    const route = (arg ?? "").replace(/^#?\/?/, "");
    await evaluate(`(location.hash = ${JSON.stringify(`#/${route}`)}, null)`);
  } else if (action === "shot") {
    const shot = await call("Page.captureScreenshot", { format: "png" });
    if (typeof shot.data !== "string") throw new Error("the debugger gave no picture");
    writeFileSync(arg, Buffer.from(shot.data, "base64"));
    console.log(`saved ${arg}`);
  } else if (action === "wait") {
    const seconds = count ?? WAIT_SECONDS;
    const end = Date.now() + seconds * 1000;
    const shows = `document.body.innerText.includes(${JSON.stringify(arg)})`;
    while (!(await evaluate(shows))) {
      if (Date.now() >= end) {
        throw new Error(`the page did not show ${JSON.stringify(arg)} within ${seconds} s`);
      }
      await pause(WAIT_EVERY_MS);
    }
  } else if (action === "fill") {
    const found = await evaluate(inPage(focusField, arg));
    if (found.problem) throw new Error(found.problem);
    await call("Input.insertText", { text: extra });
    const holds = await evaluate("document.activeElement.value");
    if (holds !== extra) {
      throw new Error(`${JSON.stringify(arg)} holds ${JSON.stringify(holds)} after the typing`);
    }
    console.log(`filled ${JSON.stringify(arg)}`);
  } else {
    const found = await evaluate(inPage(locate, arg, count));
    if (found.problem) throw new Error(found.problem);
    const { x, y } = found;
    await call("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
    await press(x, y, 1);
    let second = "";
    if (action === "dblclick") {
      await pause(DOUBLE_CLICK_PAUSE_MS);
      const met = await evaluate(inPage(controlAt, x, y));
      second = !met
        ? "; the second click met no control"
        : `; the second click met ${JSON.stringify(met.name)}${met.disabled ? ", which is disabled" : ""}`;
      await press(x, y, 2);
    }
    console.log(`clicked ${JSON.stringify(arg)}${second}`);
  }
} catch (error) {
  console.error(error.message);
  exitCode = 1;
}
socket.close();
process.exit(exitCode);
