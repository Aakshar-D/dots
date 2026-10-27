import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { checkTag, isThisFile, manifest, notesFor, PROTOTYPE_PUBKEY } from "./release.mjs";

const script = fileURLToPath(new URL("./release.mjs", import.meta.url));
/// The script's path with its folders in another case, as Windows may spell it.
const shouted = join(dirname(script).toUpperCase(), "release.mjs");

/// A checkout whose version fields read `version`, except those in `except`.
function checkout(version, except = {}) {
  const root = mkdtempSync(join(tmpdir(), "release-"));
  const v = (where) => except[where] ?? version;
  const write = (path, text) => {
    mkdirSync(join(root, path, ".."), { recursive: true });
    writeFileSync(join(root, path), text);
  };
  const pubkey = "pubkey" in except ? except.pubkey : "b3duZXIncyBrZXk=";
  write(
    "apps/dots-app/tauri.conf.json",
    JSON.stringify({ version: v("tauri"), plugins: { updater: { pubkey } } }),
  );
  write(
    "apps/dots-app/Cargo.toml",
    // CRLF, as a Windows checkout has it; a `version` in a later table is not the package's.
    `[package]\r\nname = "dots-app"\r\nversion = "${v("app")}"\r\n\r\n` +
      `[dependencies]\r\nversion = "9"\r\n`,
  );
  write("crates/dots-core/Cargo.toml", `[package]\nname = "dots-core"\nversion = "${v("core")}"\n`);
  write("ui/package.json", JSON.stringify({ name: "ui", version: v("ui") }));
  write(
    "Cargo.lock",
    `[[package]]\nname = "dots-app"\nversion = "${v("lock-app")}"\n\n` +
      `[[package]]\nname = "dots-core"\nversion = "${v("lock-core")}"\n`,
  );
  return root;
}

test("a plain tag that every field agrees with is a release", () => {
  assert.deepEqual(checkTag(checkout("0.2.0"), "v0.2.0"), {
    version: "0.2.0",
    prerelease: false,
    errors: [],
  });
});

test("a tag with a suffix is a prerelease", () => {
  const checked = checkTag(checkout("0.2.0-rc.1"), "v0.2.0-rc.1");
  assert.equal(checked.prerelease, true);
  assert.deepEqual(checked.errors, []);
});

test("every field that disagrees with the tag is named", () => {
  const checked = checkTag(checkout("0.2.0", { core: "0.1.0", "lock-app": "0.1.0" }), "v0.2.0");
  assert.deepEqual(checked.errors, [
    "crates/dots-core/Cargo.toml has 0.1.0, the tag v0.2.0",
    "Cargo.lock (dots-app) has 0.1.0, the tag v0.2.0",
  ]);
});

test("no tag is released with the prototype's updater key or with none", () => {
  // Installed copies would refuse every update signed with the owner's key.
  const prototype = checkTag(checkout("0.2.0", { pubkey: PROTOTYPE_PUBKEY }), "v0.2.0-rc.1");
  assert.deepEqual(prototype.errors.slice(-1), [
    "apps/dots-app/tauri.conf.json has the prototype's updater key; put the owner's public " +
      "key in plugins.updater.pubkey first (docs/releasing.md)",
  ]);
  const none = checkTag(checkout("0.2.0", { pubkey: undefined }), "v0.2.0");
  assert.deepEqual(none.errors, [
    "apps/dots-app/tauri.conf.json has no plugins.updater.pubkey (docs/releasing.md)",
  ]);
});

test("a tag that is not v<version> is refused", () => {
  for (const tag of ["0.2.0", "v0.2", "vnext", "v0.2.0 "]) {
    assert.equal(checkTag(checkout("0.2.0"), tag).errors.length, 1, tag);
  }
});

test("the manifest points the updater at the release's installer and its signature", () => {
  const body = manifest({
    version: "0.2.0",
    signature: "c2lnbmF0dXJl\n",
    installer: "C:/t/release/bundle/nsis/Dots_0.2.0_x64-setup.exe",
    date: new Date("2026-10-20T09:17:00Z"),
  });
  assert.deepEqual(body, {
    version: "0.2.0",
    notes: "Dots 0.2.0",
    pub_date: "2026-10-20T09:17:00.000Z",
    platforms: {
      "windows-x86_64": {
        signature: "c2lnbmF0dXJl",
        url: "https://github.com/Aakshar-D/dots/releases/download/v0.2.0/Dots_0.2.0_x64-setup.exe",
      },
    },
  });
});

test("the manifest carries the notes file when one is given", () => {
  const base = {
    version: "0.3.0",
    signature: "c2ln",
    installer: "Dots_0.3.0_x64-setup.exe",
    date: new Date("2026-10-27T09:00:00Z"),
  };
  assert.equal(manifest({ ...base, notes: "- New things\n" }).notes, "- New things");
  assert.equal(manifest(base).notes, "Dots 0.3.0");
  assert.equal(manifest({ ...base, notes: "  \n" }).notes, "Dots 0.3.0");
});

// Final review I6: the notes a promote left on master never describe a later version.
test("the notes are used only for the version they name", () => {
  const file = "<!-- dots:notes 0.3.0 -->\n- New things\n";
  assert.equal(notesFor(file, "0.3.0"), "- New things");
  assert.equal(notesFor(file, "0.3.0-rc.2"), "- New things");
  assert.equal(notesFor(file, "0.3.1"), undefined);
  assert.equal(notesFor("- unmarked", "0.3.0"), undefined);
});

test("the script knows itself by any spelling of its path", () => {
  assert.ok(isThisFile(script));
  assert.ok(isThisFile(relative(process.cwd(), script)));
  assert.ok(isThisFile(shouted, "win32"));
  assert.ok(!isThisFile(shouted, "linux"));
  assert.ok(!isThisFile(join(script, "..", "release.test.mjs")));
  assert.ok(!isThisFile(undefined));
});

test("the command line runs the check, so a bad tag fails it", () => {
  const spellings = [script, relative(process.cwd(), script)];
  if (process.platform === "win32") spellings.push(shouted);
  for (const path of spellings) {
    const ran = spawnSync(process.execPath, [path, "check", "vnext"], { encoding: "utf8" });
    assert.equal(ran.status, 1, path);
    assert.match(ran.stderr, /the tag vnext is not v<version>/, path);
  }
});
