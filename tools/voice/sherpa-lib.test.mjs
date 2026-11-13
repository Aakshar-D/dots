import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import {
  ARCHIVES,
  archiveFor,
  fetchTarget,
  guardCache,
  libDirOf,
  mergeLibs,
  parseArgs,
  unpack,
  unpackTarget,
  verify,
} from "./sherpa-lib.mjs";

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

/// A `tar` that unpacks a pretend archive into its folder: `<stem>/lib/<names>`.
function fakeTar(names, calls = []) {
  return (program, args, cwd) => {
    calls.push({ program, args, cwd });
    if (program === "tar") {
      const [, file, , dest] = args;
      const lib = join(cwd, dest, file.replace(/\.tar\.bz2$/, ""), "lib");
      mkdirSync(lib, { recursive: true });
      for (const name of names) writeFileSync(join(lib, name), name);
    } else if (program === "lipo") {
      writeFileSync(args[2], "universal");
    }
  };
}

test("unpack leaves the library folder outside the build's target folder and names it", async () => {
  const out = scratch();
  const calls = [];
  const lib = await unpack(["x86_64-pc-windows-msvc"], out, {
    download: async () => bytes,
    table,
    run: fakeTar(["onnxruntime.lib"], calls),
  });
  assert.equal(lib, libDirOf(out, "x86_64-pc-windows-msvc", table));
  assert.equal(lib, join(out, "unpacked", "x86_64-pc-windows-msvc", "pretend", "lib"));
  assert.ok(existsSync(join(lib, "onnxruntime.lib")));
  // tar is run in `out` with relative paths: GNU tar takes "D:" for a host name
  assert.deepEqual(calls[0], {
    program: "tar",
    args: ["-xjf", "pretend.tar.bz2", "-C", "unpacked/x86_64-pc-windows-msvc"],
    cwd: out,
  });
  // a second run replaces what the first left
  writeFileSync(join(lib, "stale.lib"), "old");
  unpackTarget("x86_64-pc-windows-msvc", out, { table, run: fakeTar(["onnxruntime.lib"]) });
  assert.equal(existsSync(join(lib, "stale.lib")), false);
});

test("an archive with no lib folder is refused", () => {
  const out = scratch();
  writeFileSync(join(out, pin.file), bytes);
  assert.throws(
    () => unpackTarget("x86_64-pc-windows-msvc", out, { table, run: () => {} }),
    /held no lib folder/,
  );
});

test("two Mac targets are merged into universal libraries; others cannot share a folder", async () => {
  const arm = { ...pin, file: "arm.tar.bz2" };
  const intel = { ...pin, file: "intel.tar.bz2" };
  const macs = { "aarch64-apple-darwin": arm, "x86_64-apple-darwin": intel, "x86_64-pc-windows-msvc": pin };
  const out = scratch();
  const calls = [];
  const lib = await unpack(["aarch64-apple-darwin", "x86_64-apple-darwin"], out, {
    download: async () => bytes,
    table: macs,
    run: fakeTar(["libonnxruntime.a", "libsherpa-onnx-c-api.a", "notes.txt"], calls),
  });
  assert.equal(lib, join(out, "unpacked", "universal", "lib"));
  const lipo = calls.filter((c) => c.program === "lipo");
  assert.deepEqual(
    lipo.map((c) => c.args[2]),
    [join(lib, "libonnxruntime.a"), join(lib, "libsherpa-onnx-c-api.a")],
  );
  assert.deepEqual(lipo[0].args.slice(3), [
    join(libDirOf(out, "aarch64-apple-darwin", macs), "libonnxruntime.a"),
    join(libDirOf(out, "x86_64-apple-darwin", macs), "libonnxruntime.a"),
  ]);
  await assert.rejects(
    unpack(["aarch64-apple-darwin", "x86_64-pc-windows-msvc"], scratch(), { download: async () => bytes, table: macs }),
    /only Mac targets/,
  );
  // a library one archive lacks is an error, not a half-universal build
  const a = join(scratch(), "a");
  const b = join(scratch(), "b");
  mkdirSync(a);
  mkdirSync(b);
  writeFileSync(join(a, "libx.a"), "x");
  assert.throws(() => mergeLibs([a, b], join(scratch(), "u"), { run: () => {} }), /missing/);
});
