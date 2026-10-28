// Frame leak check: runs six probes (preconnect to an IP, a name, localhost and x.localhost; a
// WebRTC STUN server; a TURN-over-TCP server) in one DevTools target of a debug Dots, with
// listeners on 127.0.0.1 that count what arrives. The CSP does not cover these, so Dots closes
// them with browser arguments (`browser_args` in apps/dots-app/src/app_frame.rs). Each probe
// waits 6 s. A target that is missing or does not answer within 10 s counts as a probe that did
// not run. Exit 1 when any probe got a hit, 2 when a probe did not run, 0 otherwise.
//
// Needs Node 22 or later (the global WebSocket). `preconnect_name` uses the public name
// leak.localtest.me, which public DNS resolves to 127.0.0.1: without that DNS it cannot hit.
//
// Usage: node tools/smoke/frame-leaks.mjs <page|iframe> <base port>
// Start a debug Dots with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS holding
// `--remote-debugging-port=9333` plus the release `browser_args(false)` (the variable replaces
// the app's own arguments). `iframe` probes an app's sandboxed frame, `page` the top page. The
// probes use ports <base port> .. <base port> + 5 on 127.0.0.1.
import dgram from "node:dgram";
import net from "node:net";
const [type, base] = [process.argv[2], Number(process.argv[3])];
const probes = [
  ["preconnect_ip", "tcp", (p) => `const l = document.createElement("link"); l.rel = "preconnect"; l.href = "http://127.0.0.1:${p}/"; document.head.append(l);`],
  ["preconnect_name", "tcp", (p) => `const l = document.createElement("link"); l.rel = "preconnect"; l.href = "http://leak.localtest.me:${p}/"; document.head.append(l);`],
  ["preconnect_localhost", "tcp", (p) => `const l = document.createElement("link"); l.rel = "preconnect"; l.href = "http://localhost:${p}/"; document.head.append(l);`],
  ["preconnect_sub_localhost", "tcp", (p) => `const l = document.createElement("link"); l.rel = "preconnect"; l.href = "http://x.localhost:${p}/"; document.head.append(l);`],
  ["stun_ip", "udp", (p) => `const pc = new RTCPeerConnection({ iceServers: [{ urls: "stun:127.0.0.1:${p}" }] }); pc.createDataChannel("x"); pc.createOffer().then((o) => pc.setLocalDescription(o)); window.__a = pc;`],
  ["turn_tcp_ip", "tcp", (p) => `const pc = new RTCPeerConnection({ iceServers: [{ urls: "turn:127.0.0.1:${p}?transport=tcp", username: "u", credential: "c" }] }); pc.createDataChannel("x"); pc.createOffer().then((o) => pc.setLocalDescription(o)); window.__b = pc;`],
];
const ANSWER_MS = 10000;
/// `promise`, or `fallback` once ANSWER_MS have passed; the timer is cleared when `promise` wins.
const within = (promise, fallback) => {
  let timer;
  const late = new Promise((r) => { timer = setTimeout(() => r(fallback), ANSWER_MS); });
  return Promise.race([promise, late]).finally(() => clearTimeout(timer));
};
const NO_ANSWER = Symbol("no answer");
let listing;
try {
  const res = await fetch("http://127.0.0.1:9333/json/list", { signal: AbortSignal.timeout(ANSWER_MS) });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  listing = await res.json();
  if (!Array.isArray(listing)) throw new Error("the list is not an array");
} catch (e) {
  console.log(`${type}: the DevTools list did not answer on port 9333 (${e.message})`);
  process.exit(2);
}
const t = listing.find((x) => x.type === type);
if (!t) {
  console.log(`no ${type} target on port 9333`);
  process.exit(2);
}
if (!t.webSocketDebuggerUrl) {
  console.log(`${type}: the target has no webSocketDebuggerUrl (another DevTools client is attached)`);
  process.exit(2);
}
const ws = new WebSocket(t.webSocketDebuggerUrl);
if ((await within(new Promise((r) => ws.addEventListener("open", () => r("open"))), "late")) !== "open") {
  console.log(`${type}: DevTools did not answer in ${ANSWER_MS / 1000} s`);
  process.exit(2);
}
let id = 0;
const evaluate = (expression) => within(new Promise((r) => {
  const me = ++id;
  const on = (m) => { const d = JSON.parse(m.data); if (d.id === me) { ws.removeEventListener("message", on); r(d.result?.result?.value ?? d.result?.exceptionDetails?.exception?.description ?? JSON.stringify(d)); } };
  ws.addEventListener("message", on);
  ws.send(JSON.stringify({ id: me, method: "Runtime.evaluate", params: { expression, returnByValue: true } }));
}), NO_ANSWER);
const out = [];
let leaked = false;
let skipped = false;
for (const [i, [name, kind, js]] of probes.entries()) {
  const port = base + i;
  let hits = 0;
  const sock = kind === "udp"
    ? dgram.createSocket("udp4").on("message", () => hits++)
    : net.createServer((s) => { hits++; s.destroy(); });
  await new Promise((r) => kind === "udp" ? sock.bind(port, "127.0.0.1", r) : sock.listen(port, "127.0.0.1", r));
  const said = await evaluate(`(() => { try { ${js(port)} return "ran"; } catch (e) { return "threw " + e.message; } })()`);
  if (said === NO_ANSWER) {
    console.log(`${name}: DevTools did not answer in ${ANSWER_MS / 1000} s; the rest are not run`);
    process.exit(2);
  }
  await new Promise((r) => setTimeout(r, 6000));
  if (hits > 0) leaked = true;
  if (said !== "ran") skipped = true;
  out.push(`${name}: ${said}, ${hits} ${kind} hit(s)`);
  sock.close();
}
console.log(type, t.url.slice(0, 55), "\n  " + out.join("\n  "));
ws.close();
setTimeout(() => process.exit(leaked ? 1 : skipped ? 2 : 0), 100);
