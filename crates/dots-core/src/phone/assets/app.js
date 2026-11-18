// The Dots phone view (phone view spec section 9). Plain script, no framework: every text from
// the PC goes into the page as text (textContent, value), never as HTML.
"use strict";

(function () {
  const POLL_MS = 4000;
  const PAIR_POLL_MS = 1500;

  const $ = (id) => document.getElementById(id);
  let pollTimer = null;
  let lastInbox = "";
  // Answers being typed, by question id, so a refresh does not wipe them.
  const drafts = new Map();
  // Cards acted on, so their buttons stay off until the list reloads without them.
  const busy = new Set();

  function say(text) {
    const status = $("status");
    status.textContent = text;
    if (text) {
      clearTimeout(say.timer);
      say.timer = setTimeout(() => (status.textContent = ""), 5000);
    }
  }

  async function call(method, path, body) {
    const init = { method, credentials: "same-origin", cache: "no-store", headers: {} };
    if (body !== undefined) {
      init.headers["Content-Type"] = "application/json";
      init.body = JSON.stringify(body);
    }
    let res;
    try {
      res = await fetch(path, init);
    } catch (e) {
      return { ok: false, status: 0, data: { error: "The PC cannot be reached. Is it on this network?" } };
    }
    let data = {};
    try {
      data = await res.json();
    } catch (e) {
      data = {};
    }
    return { ok: res.ok, status: res.status, data };
  }

  function show(view) {
    $("loading").hidden = true;
    $("pair").hidden = view !== "pair";
    $("inbox").hidden = view !== "inbox";
  }

  // ---- Pairing ----

  function defaultName() {
    const ua = navigator.userAgent || "";
    if (/iPhone/.test(ua)) return "iPhone";
    if (/iPad/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)) return "iPad";
    if (/Android/.test(ua)) return "Android phone";
    return "Phone";
  }

  function fingerprintText(hex) {
    return (hex.toUpperCase().match(/.{1,2}/g) || []).join(":");
  }

  function startPairing(token, fingerprint, lost) {
    stopPolling();
    show("pair");
    $("who").textContent = "";
    $("pair-lost").hidden = !lost;
    $("pair-name").value = $("pair-name").value || defaultName();
    $("pair-code-row").hidden = Boolean(token);
    $("pair-form").hidden = false;
    $("pair-wait").hidden = true;
    if (fingerprint) {
      $("pair-fp").hidden = false;
      $("pair-fp-text").textContent = fingerprintText(fingerprint);
    } else {
      $("pair-fp").hidden = true;
    }
    $("pair-form").onsubmit = async (event) => {
      event.preventDefault();
      const secret = token || $("pair-code").value.trim();
      if (!secret) {
        say("Type the code shown on the PC.");
        $("pair-code").focus();
        return;
      }
      $("pair-go").disabled = true;
      const res = await call("POST", "/api/pair", { secret, name: $("pair-name").value });
      $("pair-go").disabled = false;
      if (!res.ok) {
        say(res.data.error || "Pairing failed.");
        return;
      }
      // The token is used up either way: forget it.
      token = null;
      $("pair-form").hidden = true;
      $("pair-wait").hidden = false;
      $("pair-match").textContent = res.data.match;
      waitForPC(res.data.claim);
    };
  }

  function waitForPC(claim) {
    const tick = async () => {
      const res = await call("POST", "/api/pair/wait", { claim });
      const state = res.ok ? res.data.state : null;
      if (state === "waiting") {
        setTimeout(tick, PAIR_POLL_MS);
      } else if (state === "paired") {
        say("Paired.");
        loadSession();
      } else {
        const why =
          state === "refused"
            ? "The PC did not allow this phone."
            : state === "expired"
              ? "The PC did not answer in time."
              : res.data.error || "Pairing stopped.";
        say(why + " Start again on the PC.");
        startPairing(null, null, false);
      }
    };
    setTimeout(tick, PAIR_POLL_MS);
  }

  // ---- Inbox ----

  function el(tag, attrs, children) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(attrs || {})) {
      if (key === "text") node.textContent = value;
      else if (key === "className") node.className = value;
      else node.setAttribute(key, value);
    }
    for (const child of children || []) node.appendChild(child);
    return node;
  }

  function when(iso) {
    const t = Date.parse(iso);
    if (Number.isNaN(t)) return "";
    return new Date(t).toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
  }

  async function act(key, path, body, done) {
    busy.add(key);
    render(JSON.parse(lastInbox || "{}"), true);
    const res = await call("POST", path, body);
    if (!res.ok) {
      busy.delete(key);
      if (res.status === 401) return lost();
      say(res.data.error || "That did not work.");
    } else {
      say(done);
    }
    refresh(true);
  }

  function approvalCard(a) {
    const key = "approval:" + a.id;
    const off = busy.has(key);
    const approve = el("button", { type: "button", className: "primary", text: "Approve once" });
    const deny = el("button", { type: "button", text: "Deny" });
    approve.disabled = deny.disabled = off;
    approve.onclick = () => act(key, "/api/approvals/" + encodeURIComponent(a.id), { approved: true }, "Approved.");
    deny.onclick = () => act(key, "/api/approvals/" + encodeURIComponent(a.id), { approved: false }, "Denied.");
    const children = [
      el("h4", { text: a.title }),
      el("pre", { "aria-label": "What " + a.tool + " would do", text: a.input }),
    ];
    if (a.parked_note) children.push(el("p", { className: "muted small", text: a.parked_note }));
    children.push(el("p", { className: "muted small", text: "Asked " + when(a.created_at) }));
    children.push(el("div", { className: "actions" }, [approve, deny]));
    return el("li", { className: "card", "aria-label": a.title }, children);
  }

  function questionCard(q) {
    const key = "question:" + q.id;
    const off = busy.has(key);
    const boxId = "answer-" + q.id;
    const box = el("textarea", { id: boxId, maxlength: "8000" });
    box.value = drafts.get(q.id) || "";
    box.disabled = off;
    box.oninput = () => drafts.set(q.id, box.value);
    const send = el("button", { type: "button", className: "primary", text: "Send answer" });
    send.disabled = off;
    const answer = (text) => {
      if (!text.trim()) {
        say("Write an answer first.");
        return;
      }
      drafts.delete(q.id);
      act(key, "/api/questions/" + encodeURIComponent(q.id) + "/answer", { text }, "Answer sent.");
    };
    send.onclick = () => answer(box.value);
    const children = [
      el("h4", { text: q.dot_name + " asks" }),
      el("p", { text: q.text }),
    ];
    if (q.choices.length > 0) {
      const choices = q.choices.map((choice) => {
        const b = el("button", { type: "button", text: choice });
        b.disabled = off;
        b.onclick = () => answer(choice);
        return b;
      });
      children.push(el("div", { className: "actions", role: "group", "aria-label": "Answers" }, choices));
    }
    children.push(el("label", { for: boxId, className: "small", text: "Your answer" }));
    children.push(box);
    children.push(el("div", { className: "actions" }, [send]));
    return el("li", { className: "card", "aria-label": q.dot_name + " asks" }, children);
  }

  function heldCard(h) {
    const key = "held:" + h.run_id;
    const children = [el("h4", { text: h.dot_name + ": run waiting to run" }), el("p", { text: h.reason })];
    if (h.starts_after) {
      children.push(el("p", { className: "muted small", text: "Starts by itself after " + when(h.starts_after) }));
    }
    if (h.can_run_anyway) {
      const run = el("button", { type: "button", text: "Run anyway" });
      run.disabled = busy.has(key);
      run.onclick = () => act(key, "/api/runs/" + encodeURIComponent(h.run_id) + "/run-anyway", {}, "Started.");
      children.push(el("div", { className: "actions" }, [run]));
    }
    return el("li", { className: "card", "aria-label": h.dot_name + ": run waiting to run" }, children);
  }

  function fill(listId, boxId, items, card) {
    const list = $(listId);
    list.replaceChildren(...items.map(card));
    $(boxId).hidden = items.length === 0;
  }

  function render(inbox, force) {
    const text = JSON.stringify(inbox);
    if (!force && text === lastInbox) return;
    lastInbox = text;
    const approvals = inbox.approvals || [];
    const questions = inbox.questions || [];
    const held = inbox.held || [];
    // Keep focus and the caret in an answer being typed across the rebuild.
    const focused = document.activeElement && document.activeElement.id;
    const caret = focused && document.activeElement.selectionStart;
    const live = new Set();
    approvals.forEach((a) => live.add("approval:" + a.id));
    questions.forEach((q) => live.add("question:" + q.id));
    held.forEach((h) => live.add("held:" + h.run_id));
    for (const key of [...busy]) if (!live.has(key)) busy.delete(key);
    fill("approvals", "approvals-box", approvals, approvalCard);
    fill("questions", "questions-box", questions, questionCard);
    fill("held", "held-box", held, heldCard);
    $("empty").hidden = approvals.length + questions.length + held.length > 0;
    document.title = live.size > 0 ? "(" + live.size + ") Sage" : "Sage";
    if (focused) {
      const again = $(focused);
      if (again) {
        again.focus();
        if (typeof caret === "number" && again.setSelectionRange) again.setSelectionRange(caret, caret);
      }
    }
  }

  function lost() {
    lastInbox = "";
    startPairing(null, null, true);
  }

  async function refresh(force) {
    const res = await call("GET", "/api/inbox");
    if (res.status === 401) return lost();
    if (!res.ok) {
      say(res.data.error || "The Inbox could not be loaded.");
      return;
    }
    render(res.data, force);
  }

  function stopPolling() {
    clearInterval(pollTimer);
    pollTimer = null;
  }

  function startPolling() {
    stopPolling();
    pollTimer = setInterval(() => {
      if (document.visibilityState === "visible") refresh(false);
    }, POLL_MS);
  }

  async function loadSession() {
    const res = await call("GET", "/api/session");
    if (res.status === 401) return startPairing(null, null, false);
    if (!res.ok) {
      show("pair");
      say(res.data.error || "The PC cannot be reached.");
      return;
    }
    $("who").textContent = res.data.name;
    show("inbox");
    $("forget").onclick = async () => {
      if (!window.confirm("Unpair this phone? You can pair it again from the PC.")) return;
      await call("POST", "/api/forget", {});
      lost();
    };
    await refresh(true);
    startPolling();
  }

  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible" && pollTimer) refresh(false);
  });

  // A pairing link carries its one-time token and the certificate's fingerprint in the part
  // after #, which the browser never sends. Take them and wipe them from the address bar and
  // history at once.
  const params = new URLSearchParams(location.hash.slice(1));
  const token = params.get("t");
  const fingerprint = params.get("fp");
  if (location.hash) history.replaceState(null, "", location.pathname);
  if (token) startPairing(token, fingerprint, false);
  else loadSession();
})();
