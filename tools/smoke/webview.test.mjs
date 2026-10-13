// Tests of webview.mjs against a stand-in for WebView2's debugging port: an HTTP server that
// lists one page, and a WebSocket on it that answers each DevTools call as the test says.
//
//   node --test tools/smoke/
//
// The stand-in runs no page: what `click` and `wait` look for in the page is checked by the
// smoke run itself (docs/verification). Here: the calls that go out, and every way to fail.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const HELPER = fileURLToPath(new URL("./webview.mjs", import.meta.url));

/// One WebSocket frame from the start of `bytes`, or `null` while it is incomplete.
function readFrame(bytes) {
  if (bytes.length < 2) return null;
  let length = bytes[1] & 0x7f;
  let at = 2;
  if (length === 126) {
    if (bytes.length < 4) return null;
    length = bytes.readUInt16BE(2);
    at = 4;
  } else if (length === 127) {
    if (bytes.length < 10) return null;
    length = Number(bytes.readBigUInt64BE(2));
    at = 10;
  }
  const masked = (bytes[1] & 0x80) !== 0;
  const end = at + (masked ? 4 : 0) + length;
  if (bytes.length < end) return null;
  const payload = Buffer.from(bytes.subarray(end - length, end));
  if (masked) for (let i = 0; i < length; i++) payload[i] ^= bytes[at + (i % 4)];
  return { opcode: bytes[0] & 0x0f, text: payload.toString("utf8"), size: end };
}

function textFrame(text) {
  const payload = Buffer.from(text, "utf8");
  const head =
    payload.length < 126
      ? Buffer.from([0x81, payload.length])
      : Buffer.from([0x81, 126, payload.length >> 8, payload.length & 0xff]);
  return Buffer.concat([head, payload]);
}

/// Starts the stand-in. `answer(method, params)` gives the reply to one call: `{ result }`,
/// `{ error }`, or nothing for a call that is never answered. `targets` replaces the page list.
/// It is closed when the test `t` ends, passed or failed: an open server would keep the test
/// process from ending.
async function browser(t, answer, targets) {
  const calls = [];
  const sockets = new Set();
  const server = createServer((_request, response) => {
    const list = targets ?? [
      { type: "page", webSocketDebuggerUrl: `ws://127.0.0.1:${server.address().port}/page` },
    ];
    response.setHeader("content-type", "application/json");
    response.end(JSON.stringify(list));
  });
  server.on("upgrade", (request, socket) => {
    sockets.add(socket);
    const accept = createHash("sha1")
      .update(`${request.headers["sec-websocket-key"]}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)
      .digest("base64");
    socket.write(
      "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" +
        `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
    );
    let waiting = Buffer.alloc(0);
    socket.on("data", (chunk) => {
      waiting = Buffer.concat([waiting, chunk]);
      for (let frame = readFrame(waiting); frame; frame = readFrame(waiting)) {
        waiting = waiting.subarray(frame.size);
        if (frame.opcode !== 1) continue;
        const call = JSON.parse(frame.text);
        calls.push(call);
        const reply = answer(call.method, call.params);
        if (reply) socket.write(textFrame(JSON.stringify({ id: call.id, ...reply })));
      }
    });
    socket.on("error", () => {});
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  let closed;
  const close = () => {
    closed ??= new Promise((resolve) => {
      for (const socket of sockets) socket.destroy();
      server.close(resolve);
      server.closeAllConnections();
    });
    return closed;
  };
  t.after(close);
  return { port, calls, close };
}

/// Runs the helper and returns its exit code and what it printed. A helper that does not end
/// is stopped after 20 s, and its code is then `null`.
function helper(...args) {
  return new Promise((resolve) => {
    const options = { timeout: 20_000 };
    execFile(process.execPath, [HELPER, ...args.map(String)], options, (error, stdout, stderr) => {
      resolve({ code: error ? error.code : 0, stdout, stderr });
    });
  });
}

/// The reply to `Runtime.evaluate` when the page's expression gave `value`, and when it threw.
const evaluated = (value) => ({ result: { result: { value: JSON.stringify({ value }) } } });
const threw = (thrown) => ({ result: { result: { value: JSON.stringify({ thrown }) } } });

/// `text` is one line, and not a stack trace.
function assertOneLine(text) {
  assert.match(text, /^[^\n]+\n$/);
  assert.doesNotMatch(text, /^\s+at /m);
}

test("eval prints the value the page gave", async (t) => {
  const fake = await browser(t, () => evaluated({ port: 47321, server_error: null }));
  const run = await helper(fake.port, "eval", 'invoke("status")');
  await fake.close();
  assert.deepEqual(run, {
    code: 0,
    stdout: '{\n  "port": 47321,\n  "server_error": null\n}\n',
    stderr: "",
  });
  assert.equal(fake.calls[0].method, "Runtime.evaluate");
  assert.ok(fake.calls[0].params.expression.includes('invoke("status")'));
});

test("a call the debugger refuses is an error, not 'undefined'", async (t) => {
  const refusal = { error: { code: -32000, message: "Execution context was destroyed." } };
  const fake = await browser(t, () => refusal);
  const run = await helper(fake.port, "eval", "1 + 1");
  await fake.close();
  assert.deepEqual(run, {
    code: 1,
    stdout: "",
    stderr: "the debugger refused Runtime.evaluate: Execution context was destroyed.\n",
  });
});

test("a command the backend refuses is reported with its kind and message", async (t) => {
  // A refused `invoke` rejects with an object, which the debugger describes only as "Object":
  // the page hands over what was thrown itself.
  const fake = await browser(t, () => threw({ kind: "not_found", message: "dot x not found" }));
  const run = await helper(fake.port, "eval", "invoke('get_dot', { id: 'x' })");
  assert.deepEqual(run, {
    code: 1,
    stdout: "",
    stderr: 'the page threw: {"kind":"not_found","message":"dot x not found"}\n',
  });
  assert.ok(fake.calls[0].params.expression.includes("catch"));
});

test("an expression the page cannot run is reported", async (t) => {
  const fake = await browser(t, () => ({
    result: {
      exceptionDetails: {
        text: "Uncaught",
        exception: { description: "SyntaxError: Unexpected token ')'" },
      },
    },
  }));
  const run = await helper(fake.port, "eval", "invoke(");
  assert.deepEqual(run, {
    code: 1,
    stdout: "",
    stderr: "the page threw: \"SyntaxError: Unexpected token ')'\"\n",
  });
});

test("shot saves the picture the debugger gave", async (t) => {
  const dir = mkdtempSync(join(tmpdir(), "webview-test-"));
  const file = join(dir, "page.png");
  const fake = await browser(t, () => ({ result: { data: Buffer.from("png").toString("base64") } }));
  const run = await helper(fake.port, "shot", file);
  await fake.close();
  assert.equal(fake.calls[0].method, "Page.captureScreenshot");
  assert.deepEqual(run, { code: 0, stdout: `saved ${file}\n`, stderr: "" });
  assert.equal(readFileSync(file, "utf8"), "png");
  rmSync(dir, { recursive: true });
});

test("shot says why a capture failed and writes no file", async (t) => {
  const dir = mkdtempSync(join(tmpdir(), "webview-test-"));
  const file = join(dir, "page.png");
  const fake = await browser(t, () => ({ error: { code: -32000, message: "Unable to capture" } }));
  const run = await helper(fake.port, "shot", file);
  await fake.close();
  assert.deepEqual(run, {
    code: 1,
    stdout: "",
    stderr: "the debugger refused Page.captureScreenshot: Unable to capture\n",
  });
  assert.ok(!existsSync(file));
  rmSync(dir, { recursive: true });
});

test("an action without its argument is a usage error, before anything is asked", async (t) => {
  const fake = await browser(t, () => evaluated(null));
  const incomplete = [["shot"], ["eval"], ["click"], ["dblclick"], ["wait"], ["fill"], ["fill", "Name"]];
  // And what is not a number where one is expected, and an action that does not exist.
  for (const args of [...incomplete, ["click", "Approve", "second"], ["wait", "Done", "0"], ["press"]]) {
    const run = await helper(fake.port, ...args);
    assert.equal(run.code, 2, args.join(" "));
    assert.equal(run.stdout, "", args.join(" "));
    assert.match(run.stderr, /^usage: /, args.join(" "));
  }
  await fake.close();
  assert.deepEqual(fake.calls, []);
});

test("a port nothing listens on is one line that says how to open it", async (t) => {
  // A port that was free a moment ago.
  const fake = await browser(t, () => undefined);
  await fake.close();
  const run = await helper(fake.port, "text");
  assert.equal(run.code, 1);
  assert.equal(run.stdout, "");
  assertOneLine(run.stderr);
  assert.ok(run.stderr.includes(`nothing answers on debugging port ${fake.port}`), run.stderr);
  assert.ok(run.stderr.includes(`--remote-debugging-port=${fake.port}`), run.stderr);
});

test("a debugging port without a page is one line", async (t) => {
  const fake = await browser(t, () => undefined, [{ type: "service_worker" }]);
  const run = await helper(fake.port, "text");
  await fake.close();
  assert.deepEqual(run, {
    code: 1,
    stdout: "",
    stderr: `no page on debugging port ${fake.port}\n`,
  });
});

test("click presses and releases the left button once where the control is", async (t) => {
  const fake = await browser(t, (method) =>
    method === "Runtime.evaluate" ? evaluated({ x: 410, y: 96 }) : { result: {} },
  );
  const run = await helper(fake.port, "click", "Mark reviewed", 2);
  await fake.close();
  assert.deepEqual(run, { code: 0, stdout: 'clicked "Mark reviewed"\n', stderr: "" });
  const [find, ...mouse] = fake.calls;
  // The name and the number reach the page as data, not as code.
  assert.ok(find.params.expression.includes('"Mark reviewed"'), find.params.expression);
  assert.deepEqual(
    mouse.map((call) => [call.method, call.params.type, call.params.clickCount ?? null]),
    [
      ["Input.dispatchMouseEvent", "mouseMoved", null],
      ["Input.dispatchMouseEvent", "mousePressed", 1],
      ["Input.dispatchMouseEvent", "mouseReleased", 1],
    ],
  );
  for (const call of mouse) {
    assert.deepEqual([call.params.x, call.params.y], [410, 96]);
  }
});

test("dblclick clicks twice at one point and names what the second click met", async (t) => {
  let finds = 0;
  const fake = await browser(t, (method) => {
    if (method !== "Runtime.evaluate") return { result: {} };
    finds += 1;
    // First the control is found; after the first click the page is asked what is there now.
    return finds === 1
      ? evaluated({ x: 300, y: 200 })
      : evaluated({ name: "Approve", disabled: true });
  });
  const run = await helper(fake.port, "dblclick", "Approve", 1);
  await fake.close();
  assert.deepEqual(run, {
    code: 0,
    stdout: 'clicked "Approve"; the second click met "Approve", which is disabled\n',
    stderr: "",
  });
  const presses = fake.calls
    .filter((call) => call.method === "Input.dispatchMouseEvent" && call.params.type !== "mouseMoved")
    .map((call) => [call.params.type, call.params.clickCount, call.params.x, call.params.y]);
  assert.deepEqual(presses, [
    ["mousePressed", 1, 300, 200],
    ["mouseReleased", 1, 300, 200],
    ["mousePressed", 2, 300, 200],
    ["mouseReleased", 2, 300, 200],
  ]);
});

test("click reports what the page says about a control it cannot click", async (t) => {
  const fake = await browser(t, () =>
    evaluated({ problem: 'no control named "Aprove" (the page has: "Approve", "Deny")' }),
  );
  const run = await helper(fake.port, "click", "Aprove");
  await fake.close();
  assert.deepEqual(run, {
    code: 1,
    stdout: "",
    stderr: 'no control named "Aprove" (the page has: "Approve", "Deny")\n',
  });
  assert.equal(fake.calls.length, 1);
});

test("fill types the text into the field and checks that the field holds it", async (t) => {
  let asked = 0;
  const fake = await browser(t, (method) => {
    if (method !== "Runtime.evaluate") return { result: {} };
    // First the field gets the focus; after the typing its value is read back.
    return ++asked === 1 ? evaluated({}) : evaluated("C:/tools/claude.exe");
  });
  const run = await helper(fake.port, "fill", "Program path", "C:/tools/claude.exe");
  assert.deepEqual(run, { code: 0, stdout: 'filled "Program path"\n', stderr: "" });
  assert.ok(fake.calls[0].params.expression.includes('"Program path"'));
  assert.deepEqual(
    [fake.calls[1].method, fake.calls[1].params],
    ["Input.insertText", { text: "C:/tools/claude.exe" }],
  );

  const refused = await helper(fake.port, "fill", "Program path", "another");
  assert.deepEqual(refused, {
    code: 1,
    stdout: "",
    stderr: '"Program path" holds "C:/tools/claude.exe" after the typing\n',
  });
});

test("wait ends when the page shows the text, and fails when it never does", async (t) => {
  let asked = 0;
  const shown = await browser(t, () => evaluated(++asked >= 3));
  const found = await helper(shown.port, "wait", "Succeeded", 5);
  await shown.close();
  assert.deepEqual(found, { code: 0, stdout: "", stderr: "" });
  assert.equal(asked, 3);

  const never = await browser(t, () => evaluated(false));
  const missed = await helper(never.port, "wait", "Succeeded", 1);
  await never.close();
  assert.deepEqual(missed, {
    code: 1,
    stdout: "",
    stderr: 'the page did not show "Succeeded" within 1 s\n',
  });
});

test("a page that closes before it answers is reported", async (t) => {
  // As after invoke("restart_app"): the call is never answered and the socket goes away.
  const fake = await browser(t, () => {
    setTimeout(() => fake.close(), 50);
    return undefined;
  });
  const run = await helper(fake.port, "eval", 'invoke("restart_app")');
  assert.deepEqual(run, {
    code: 1,
    stdout: "",
    stderr: "the page closed before it answered\n",
  });
});
