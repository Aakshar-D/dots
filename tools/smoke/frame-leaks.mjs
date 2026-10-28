// Frame leak check: runs six probes (preconnect to an IP, a name, localhost and x.localhost; a
// WebRTC STUN server; a TURN-over-TCP server) in one DevTools target of a debug Dots, with
// listeners on 127.0.0.1 that count what arrives. The CSP does not cover these, so Dots closes
// them with browser arguments (`browser_args` in apps/dots-app/src/app_frame.rs). Each probe
// waits 6 s. Exit 1 when any probe got a hit, 2 when a probe did not run, 0 otherwise.
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
const t = (await (await fetch("http://127.0.0.1:9333/json/list")).json()).find((x) => x.type === type);
const ws = new WebSocket(t.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0;
const evaluate = (expression) => new Promise((r) => {
  const me = ++id;
  const on = (m) => { const d = JSON.parse(m.data); if (d.id === me) { ws.removeEventListener("message", on); r(d.result?.result?.value ?? d.result?.exceptionDetails?.exception?.description ?? JSON.stringify(d)); } };
  ws.addEventListener("message", on);
  ws.send(JSON.stringify({ id: me, method: "Runtime.evaluate", params: { expression, returnByValue: true } }));
});
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
  await new Promise((r) => setTimeout(r, 6000));
  if (hits > 0) leaked = true;
  if (said !== "ran") skipped = true;
  out.push(`${name}: ${said}, ${hits} ${kind} hit(s)`);
  sock.close();
}
console.log(type, t.url.slice(0, 55), "\n  " + out.join("\n  "));
ws.close();
setTimeout(() => process.exit(leaked ? 1 : skipped ? 2 : 0), 100);
