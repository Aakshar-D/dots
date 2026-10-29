// Dots' client for an app's screen (app library spec section 5). The `dots-app` protocol puts it
// at the top of the app's index.html. The page talks to Dots only through the MessagePort the
// host sends once it has loaded: `dots.call(tool, input)` answers with the tool's result or
// rejects with its error, `dots.onChange(cb)` hears that the app's data changed, and the
// theme's colours arrive as CSS variables (`--dots-<token>`) on the root element, with
// `data-theme` set to "light" or "dark" (`dots.onTheme(cb)` hears a change).
// `dots.openRun(runId)` opens, in Dots, the conversation of a run the app started; it rejects
// for any other run.
(() => {
  let port = null;
  let next = 0;
  const waiting = new Map();
  const queued = [];
  const changes = new Set();
  const themes = new Set();
  const root = document.documentElement;
  const each = (set, arg) => {
    for (const cb of set) {
      try {
        cb(arg);
      } catch (e) {
        console.error(e);
      }
    }
  };
  const theme = (msg) => {
    for (const [name, value] of Object.entries(msg.tokens)) {
      root.style.setProperty(`--dots-${name}`, value);
    }
    root.dataset.theme = msg.mode;
    root.style.colorScheme = msg.mode;
    each(themes, msg.mode);
  };
  const receive = (event) => {
    const msg = event.data;
    if (msg === null || typeof msg !== "object") return;
    if (msg.type === "result") {
      const call = waiting.get(msg.id);
      if (call === undefined) return;
      waiting.delete(msg.id);
      if (msg.ok) call.resolve(msg.result);
      else call.reject(new Error(String(msg.error)));
    } else if (msg.type === "theme") {
      theme(msg);
    } else if (msg.type === "changed") {
      each(changes);
    }
  };
  window.addEventListener("message", (event) => {
    // One port, from the window that framed this page.
    if (port !== null || event.source !== window.parent) return;
    if (event.data === null || typeof event.data !== "object") return;
    if (event.data.type !== "dots:connect" || event.ports.length !== 1) return;
    port = event.ports[0];
    port.onmessage = receive;
    for (const msg of queued.splice(0)) deliver(msg);
  });
  // A message the port refuses (one that cannot be cloned) rejects only its own call, so the
  // calls queued before or after it still go out.
  const deliver = (msg) => {
    try {
      port.postMessage(msg);
    } catch (error) {
      const call = waiting.get(msg.id);
      if (call === undefined) return;
      waiting.delete(msg.id);
      call.reject(new Error(String(error)));
    }
  };
  const send = (msg) => (port === null ? queued.push(msg) : deliver(msg));
  // A request the host answers with a result of the same id.
  const ask = (msg) =>
    new Promise((resolve, reject) => {
      next += 1;
      waiting.set(next, { resolve, reject });
      send({ ...msg, id: next });
    });
  const dots = Object.freeze({
    call(tool, input = {}) {
      return ask({ type: "call", tool: String(tool), input });
    },
    openRun(runId) {
      return ask({ type: "open_run", run_id: String(runId) });
    },
    onChange(cb) {
      changes.add(cb);
      return () => changes.delete(cb);
    },
    onTheme(cb) {
      themes.add(cb);
      return () => themes.delete(cb);
    },
  });
  Object.defineProperty(window, "dots", { value: dots, enumerable: true });
})();
