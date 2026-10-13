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
//
// In `eval`, `invoke` is the Tauri bridge: invoke("status"), invoke("run_now", {dotId}).
// After invoke("restart_app") the page closes before it answers; the script then says so and
// exits with code 1, which is the expected outcome of that one call.
// Needs Node 22 or newer (global fetch and WebSocket).
import { writeFileSync } from "node:fs";

const [port, action, arg] = process.argv.slice(2);
if (!port || !["eval", "text", "goto", "shot"].includes(action)) {
  console.error('usage: webview.mjs <port> eval "<expr>" | text | goto <route> | shot <file.png>');
  process.exit(2);
}

const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page");
if (!page) throw new Error(`no page on debugging port ${port}`);
const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  socket.onopen = resolve;
  socket.onerror = () => reject(new Error("could not connect to the page"));
});

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
const send = (method, params = {}) =>
  new Promise((resolve) => {
    waiting.set(++lastId, resolve);
    socket.send(JSON.stringify({ id: lastId, method, params }));
  });

/// Evaluates an async expression in the page and returns its value (JSON-encoded there).
async function evaluate(expression) {
  const reply = await send("Runtime.evaluate", {
    expression: `(async () => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      return JSON.stringify(await (async () => (${expression}))());
    })()`,
    awaitPromise: true,
    returnByValue: true,
  });
  if (reply.closed) throw new Error("the page closed before it answered");
  const failure = reply.result?.exceptionDetails;
  if (failure) {
    const thrown = failure.exception?.value ?? failure.exception?.description ?? failure.text;
    throw new Error(`the page threw: ${JSON.stringify(thrown)}`);
  }
  const value = reply.result?.result?.value;
  return value === undefined ? undefined : JSON.parse(value);
}

let exitCode = 0;
try {
  if (action === "eval") {
    console.log(JSON.stringify(await evaluate(arg), null, 2));
  } else if (action === "text") {
    console.log(await evaluate("document.body.innerText"));
  } else if (action === "goto") {
    const route = (arg ?? "").replace(/^#?\/?/, "");
    await evaluate(`(location.hash = ${JSON.stringify(`#/${route}`)}, null)`);
  } else {
    const shot = await send("Page.captureScreenshot", { format: "png" });
    writeFileSync(arg, Buffer.from(shot.result.data, "base64"));
    console.log(`saved ${arg}`);
  }
} catch (error) {
  console.error(error.message);
  exitCode = 1;
}
socket.close();
process.exit(exitCode);
