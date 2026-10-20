import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { checkTag, manifest } from "./release.mjs";

/// A checkout whose version fields read `version`, except those in `except`.
function checkout(version, except = {}) {
  const root = mkdtempSync(join(tmpdir(), "release-"));
  const v = (where) => except[where] ?? version;
  const write = (path, text) => {
    mkdirSync(join(root, path, ".."), { recursive: true });
    writeFileSync(join(root, path), text);
  };
  write("apps/dots-app/tauri.conf.json", JSON.stringify({ version: v("tauri") }));
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
