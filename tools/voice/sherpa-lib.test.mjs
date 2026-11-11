import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { ARCHIVES, archiveFor, fetchTarget, guardCache, parseArgs, verify } from "./sherpa-lib.mjs";

const bytes = Buffer.from("a pretend archive");
const pin = { file: "pretend.tar.bz2", size: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") };
const table = { "x86_64-pc-windows-msvc": pin };
const scratch = () => mkdtempSync(join(tmpdir(), "sherpa-lib-test-"));

test("every shipped or checked target has a pin with a hash, a size and a file name", () => {
  for (const target of [
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-pc-windows-msvc",
  ]) {
    const entry = archiveFor(target);
    assert.match(entry.sha256, /^[0-9a-f]{64}$/, target);
    assert.ok(Number.isInteger(entry.size) && entry.size > 0, target);
    assert.match(entry.file, /^sherpa-onnx-v2\.0\.1-.*-lib\.tar\.bz2$/, target);
  }
  assert.equal(new Set(Object.values(ARCHIVES).map((entry) => entry.file)).size, 4);
});

test("an unknown target is refused with the known ones named", () => {
  assert.throws(() => archiveFor("riscv64-unknown-linux-gnu"), /no sherpa-onnx archive.*riscv64.*x86_64-pc-windows-msvc/);
  assert.throws(() => archiveFor("toString"), /no sherpa-onnx archive/);
});

test("verify accepts the right bytes and rejects a wrong hash or size", () => {
  assert.equal(verify(bytes, pin).ok, true);
  const other = Buffer.from("a pretend archivf");
  const wrongHash = verify(other, pin);
  assert.equal(wrongHash.ok, false);
  assert.match(wrongHash.problem, /sha256/);
  const wrongSize = verify(Buffer.concat([bytes, Buffer.from("!")]), pin);
  assert.equal(wrongSize.ok, false);
  assert.match(wrongSize.problem, /size/);
});

test("verify reads a file by its path", () => {
  const dir = scratch();
  writeFileSync(join(dir, "f"), bytes);
  assert.equal(verify(join(dir, "f"), pin).ok, true);
});

test("a download with the wrong hash writes nothing and says what it expected and got", async () => {
  const out = scratch();
  const wrong = Buffer.from("a pretend archivf");
  await assert.rejects(
    fetchTarget("x86_64-pc-windows-msvc", out, { table, download: async () => wrong }),
    (error) => error.message.includes(pin.sha256) && error.message.includes(verify(wrong, pin).sha256),
  );
  assert.deepEqual(readdirSync(out), []);
});

test("a download with the right hash is written under the exact name", async () => {
  const out = scratch();
  const urls = [];
  const result = await fetchTarget("x86_64-pc-windows-msvc", out, {
    table,
    download: async (url) => (urls.push(url), bytes),
  });
  assert.equal(result, "fetched");
  assert.deepEqual(readdirSync(out), [pin.file]);
  assert.deepEqual(urls, [`https://github.com/k2-fsa/sherpa-onnx/releases/download/v2.0.1/${pin.file}`]);
});

test("a good archive already in the folder is not downloaded again", async () => {
  const out = scratch();
  writeFileSync(join(out, pin.file), bytes);
  const result = await fetchTarget("x86_64-pc-windows-msvc", out, {
    table,
    download: async () => assert.fail("must not download"),
  });
  assert.equal(result, "cached");
});

test("a bad archive in the folder is replaced by a verified download, or removed", async () => {
  const out = scratch();
  writeFileSync(join(out, pin.file), "tampered");
  await assert.rejects(fetchTarget("x86_64-pc-windows-msvc", out, { table, download: async () => Buffer.from("x") }));
  assert.equal(existsSync(join(out, pin.file)), false);
  writeFileSync(join(out, pin.file), "tampered");
  assert.equal(await fetchTarget("x86_64-pc-windows-msvc", out, { table, download: async () => bytes }), "fetched");
  assert.equal(verify(join(out, pin.file), pin).ok, true);
});

test("the build's own cache is accepted when clean or absent and refused when unverified", () => {
  const target = scratch();
  assert.deepEqual(guardCache(target, table), []);
  const cache = join(target, "sherpa-onnx-prebuilt");
  mkdirSync(join(cache, "pretend"), { recursive: true });
  assert.equal(guardCache(target, table).length, 1, "unpacked folder without a verified archive");
  writeFileSync(join(cache, pin.file), bytes);
  mkdirSync(join(cache, "pretend"), { recursive: true });
  assert.deepEqual(guardCache(target, table), []);
  writeFileSync(join(cache, pin.file), "tampered");
  assert.equal(guardCache(target, table).length, 2);
  writeFileSync(join(cache, "other.tar.bz2"), "unknown");
  assert.equal(guardCache(target, table).length, 3);
});

test("the command line takes several targets", () => {
  assert.deepEqual(parseArgs(["--target", "a", "--target", "b", "--out", "dir"]), {
    targets: ["a", "b"],
    out: "dir",
    targetDir: null,
  });
  assert.throws(() => parseArgs(["--bogus", "x"]), /unknown option/);
});
