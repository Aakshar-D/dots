// The install commands of the README (`install.ps1`, `install.sh`) against a manifest made as
// a release makes it: each must find the file for its platform, whatever the version. Run with
// `node --test tools/release/install.test.mjs`. A script whose shell this machine lacks is
// skipped.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const root = join(fileURLToPath(import.meta.url), "..", "..", "..");
const releases = "https://github.com/Aakshar-D/dots/releases/download";

/// A manifest as `release.mjs manifest` writes it (pretty-printed, signature before url), for
/// the version `v`, with the platforms `platforms` (name to file).
function manifestFile(v, platforms) {
  const entries = Object.fromEntries(
    Object.entries(platforms).map(([name, file]) => [
      name,
      { signature: "c2lnbmF0dXJl", url: `${releases}/v${v}/${file}` },
    ]),
  );
  const manifest = { version: v, notes: 'Notes with "quotes" and {braces}.', platforms: entries };
  const file = join(mkdtempSync(join(tmpdir(), "sage-install-")), "latest.json");
  writeFileSync(file, JSON.stringify(manifest, null, 2) + "\n");
  return file;
}

const all = (v) => ({
  "windows-x86_64": `Sage_${v}_x64-setup.exe`,
  "windows-aarch64": `Sage_${v}_arm64-setup.exe`,
  "darwin-aarch64": `Sage_${v}_universal.app.tar.gz`,
  "darwin-x86_64": `Sage_${v}_universal.app.tar.gz`,
});

/// Runs `command` as a dry run on the manifest `file`; undefined when the shell is missing.
function dryRun(command, args, file, env = {}) {
  const run = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, SAGE_DRY_RUN: "1", SAGE_MANIFEST_FILE: file, ...env },
  });
  if (run.error?.code === "ENOENT") return undefined;
  return { status: run.status, out: `${run.stdout}${run.stderr}`.trim() };
}

test("the Mac command finds the app of whatever version is newest", (t) => {
  for (const v of ["0.5.4", "1.12.0"]) {
    const run = dryRun("sh", ["install.sh"], manifestFile(v, all(v)));
    if (!run) return t.skip("no sh here");
    assert.equal(run.status, 0, run.out);
    assert.equal(run.out, `Sage ${v}: ${releases}/v${v}/Sage_${v}_universal.app.tar.gz`);
  }
});

test("the Mac command installs nothing from a release without a Mac app, or from elsewhere", (t) => {
  const none = dryRun(
    "sh",
    ["install.sh"],
    manifestFile("0.5.4", { "windows-x86_64": "Sage_0.5.4_x64-setup.exe" }),
  );
  if (!none) return t.skip("no sh here");
  assert.notEqual(none.status, 0);
  assert.match(none.out, /has no Mac app/);

  const file = manifestFile("0.5.4", all("0.5.4"));
  const text = JSON.stringify({
    version: "0.5.4",
    platforms: {
      "darwin-aarch64": { signature: "x", url: "https://example.com/Sage.app.tar.gz" },
      "darwin-x86_64": { signature: "x", url: "https://example.com/Sage.app.tar.gz" },
    },
  });
  writeFileSync(file, text);
  const elsewhere = dryRun("sh", ["install.sh"], file);
  assert.notEqual(elsewhere.status, 0);
  assert.match(elsewhere.out, /outside https:\/\/github\.com\/Aakshar-D\/dots\/releases/);
});

const powershell = ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "install.ps1"];

test("the Windows command finds the installer for the PC's processor", (t) => {
  const v = "1.12.0";
  const file = manifestFile(v, all(v));
  const x64 = dryRun("powershell", powershell, file, { SAGE_ARCH: "x64" });
  if (!x64) return t.skip("no powershell here");
  assert.equal(x64.status, 0, x64.out);
  assert.equal(x64.out, `Sage ${v}: ${releases}/v${v}/Sage_${v}_x64-setup.exe`);
  const arm = dryRun("powershell", powershell, file, { SAGE_ARCH: "arm64" });
  assert.equal(arm.out, `Sage ${v}: ${releases}/v${v}/Sage_${v}_arm64-setup.exe`);

  // A release without the ARM64 installer gives an ARM64 PC the x64 one, which runs there.
  const only = manifestFile(v, { "windows-x86_64": `Sage_${v}_x64-setup.exe` });
  const fallback = dryRun("powershell", powershell, only, { SAGE_ARCH: "arm64" });
  assert.equal(fallback.out, `Sage ${v}: ${releases}/v${v}/Sage_${v}_x64-setup.exe`);
});

test("the Windows command, run as the README gives it, leaves the PowerShell as it was", (t) => {
  const file = manifestFile("0.5.4", all("0.5.4"));
  const piped = [
    "-NoProfile",
    "-Command",
    'Get-Content -Raw install.ps1 | iex; "left: $ErrorActionPreference $($null -eq $manifest)"',
  ];
  const run = dryRun("powershell", piped, file, { SAGE_ARCH: "x64" });
  if (!run) return t.skip("no powershell here");
  assert.equal(run.status, 0, run.out);
  assert.match(run.out, /^Sage 0\.5\.4: \S+x64-setup\.exe\s+left: Continue True$/);
});

test("the Windows command runs nothing from outside this repository's releases", (t) => {
  const file = manifestFile("0.5.4", all("0.5.4"));
  writeFileSync(
    file,
    JSON.stringify({
      version: "0.5.4",
      platforms: { "windows-x86_64": { signature: "x", url: "https://example.com/setup.exe" } },
    }),
  );
  const run = dryRun("powershell", powershell, file, { SAGE_ARCH: "x64" });
  if (!run) return t.skip("no powershell here");
  assert.notEqual(run.status, 0);
  assert.match(run.out, /outside https:\/\/github\.com\/Aakshar-D\/dots\/releases/);
});
