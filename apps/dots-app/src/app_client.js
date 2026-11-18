// Sage's client for an app's screen (app library spec section 5). The `dots-app` protocol puts it
// at the top of the app's index.html. The page talks to Sage only through the MessagePort the
// host sends once it has loaded: `dots.call(tool, input)` answers with the tool's result or
// rejects with its error, `dots.onChange(cb)` hears that the app's data changed, and the
// theme's colours arrive as CSS variables (`--dots-<token>`) on the root element, with
// `data-theme` set to "light" or "dark" (`dots.onTheme(cb)` hears a change).
// `dots.openRun(runId)` opens, in Sage, the conversation of a run the app started; it rejects
// for any other run. The frame cannot start a download itself, so `dots.saveFile(name, base64)`
// hands Dots the file: the person picks where in the Save dialog, and it answers true when they
// saved it, false when they cancelled. A page that starts a download the usual way (a `blob:`
// or `data:` link, `window.open`) is caught at the end of this file and saved the same way.
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
    saveFile(name, base64) {
      return ask({ type: "save_file", name: String(name), data: String(base64) });
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

  // A page that offers a file the usual way (a link with `download`, a click on a link it
  // never put on the page, `window.open`) gets nothing from the frame, which refuses every
  // download. So a `blob:` or `data:` address used in one of those ways is caught here and its
  // file goes through `saveFile`: the person gets the Save dialog without the page having been
  // written for it. Any other address is left as it was.
  const { URL: Urls, Blob: Blobs, HTMLAnchorElement: Anchor, TextEncoder: Encoder } = window;
  // A `blob:` address cannot be read back (the page connects nowhere), so the blob behind each
  // one the page makes is kept here until the page revokes the address.
  const blobs = new Map();
  if (Urls && Blobs && typeof Urls.createObjectURL === "function") {
    const create = Urls.createObjectURL.bind(Urls);
    const revoke = Urls.revokeObjectURL.bind(Urls);
    Urls.createObjectURL = (object) => {
      const url = create(object);
      if (object instanceof Blobs) blobs.set(url, object);
      return url;
    };
    Urls.revokeObjectURL = (url) => {
      blobs.delete(String(url));
      revoke(url);
    };
  }
  const base64 = (bytes) => {
    let text = "";
    for (let i = 0; i < bytes.length; i += 0x8000) {
      text += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    }
    return window.btoa(text);
  };
  // The file at a `blob:` or `data:` address: its type and a promise of its base64. Null for
  // any other address, and for a `blob:` one this page did not make or has revoked.
  const fileAt = (href) => {
    if (typeof href !== "string") return null;
    if (href.startsWith("blob:")) {
      const blob = blobs.get(href);
      if (blob === undefined) return null;
      return {
        type: blob.type,
        data: blob.arrayBuffer().then((buffer) => base64(new Uint8Array(buffer))),
      };
    }
    const parts = /^data:([^,]*),(.*)$/s.exec(href);
    if (parts === null) return null;
    const about = parts[1].split(";");
    const type = about[0].trim().toLowerCase();
    if (about.slice(1).some((part) => part.trim().toLowerCase() === "base64")) {
      return { type, data: Promise.resolve(parts[2]) };
    }
    return {
      type,
      data: new Promise((resolve) =>
        resolve(base64(new Encoder().encode(decodeURIComponent(parts[2])))),
      ),
    };
  };
  const ENDINGS = {
    "application/pdf": "pdf",
    "application/json": "json",
    "application/zip": "zip",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet": "xlsx",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document": "docx",
    "image/png": "png",
    "image/jpeg": "jpg",
    "image/svg+xml": "svg",
    "text/csv": "csv",
    "text/html": "html",
    "text/markdown": "md",
    "text/plain": "txt",
  };
  // The name to save under: the link's, or "download", with the type's ending when it has none.
  const named = (name, type) => {
    const base = typeof name === "string" && name.trim() !== "" ? name.trim() : "download";
    const ending = ENDINGS[String(type).split(";")[0].trim().toLowerCase()];
    return ending === undefined || /\.[A-Za-z0-9]{1,8}$/.test(base) ? base : `${base}.${ending}`;
  };
  // Hands Sage the file at `href`, to save as `name`. False when there is no file there to
  // hand over; the address is then left to do what it did before.
  const offer = (href, name) => {
    const file = fileAt(href);
    if (file === null) return false;
    file.data
      .then((data) => dots.saveFile(named(name, file.type), data))
      .catch((error) => console.error(error));
    return true;
  };
  // A click on a link in the page, after the page's own handlers: one of them that saved the
  // file itself has said so by preventing the click.
  window.addEventListener("click", (event) => {
    if (event.defaultPrevented) return;
    const target = event.target;
    const link = target && typeof target.closest === "function" ? target.closest("a[href]") : null;
    if (link === null) return;
    if (offer(link.getAttribute("href"), link.getAttribute("download"))) event.preventDefault();
  });
  // `link.click()` on a link that is not on the page, the common way to start a download from
  // script: no click reaches the window for it.
  if (Anchor && Anchor.prototype) {
    const click = Anchor.prototype.click;
    Anchor.prototype.click = function () {
      if (
        !this.isConnected &&
        offer(this.getAttribute("href"), this.getAttribute("download"))
      ) {
        return;
      }
      click.call(this);
    };
  }
  // `window.open` on a file's address: the frame opens no window.
  if (typeof window.open === "function") {
    const open = window.open;
    window.open = function (url, ...rest) {
      return offer(String(url), null) ? null : open.call(window, url, ...rest);
    };
  }
})();
