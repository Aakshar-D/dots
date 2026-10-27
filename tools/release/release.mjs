// The release workflow's two steps that are Dots' own (updates spec §4), kept out of the
// workflow file so they are tested (`node --test tools/release/release.test.mjs`).
//
//   node tools/release/release.mjs check <tag>
//       Fails unless the tag, without its "v", is the version in every version field.
//       Prints "prerelease=true" or "prerelease=false", and appends the same line to
//       $GITHUB_OUTPUT when it is set.
//   node tools/release/release.mjs manifest <tag> <installer> <out>
//       Writes the updater manifest (latest.json) for the NSIS installer <installer>, whose
//       signature is <installer>.sig, to <out>.

import { appendFileSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/// Where the release's files are downloaded from.
const DOWNLOADS = "https://github.com/Aakshar-D/dots/releases/download";
/// The updater's name for this platform's entry in the manifest.
const PLATFORM = "windows-x86_64";

/// The version of every field the version lives in, by where it is.
export function versionFields(root) {
  const read = (path) => readFileSync(join(root, path), "utf8").replace(/\r\n/g, "\n");
  const cargo = (path) => read(path).match(/^\[package\][^[]*?^version = "([^"]*)"/m)?.[1];
  const locked = (name) =>
    read("Cargo.lock").match(new RegExp(`^name = "${name}"\\nversion = "([^"]*)"`, "m"))?.[1];
  return {
    "apps/dots-app/tauri.conf.json": JSON.parse(read("apps/dots-app/tauri.conf.json")).version,
    "apps/dots-app/Cargo.toml": cargo("apps/dots-app/Cargo.toml"),
    "crates/dots-core/Cargo.toml": cargo("crates/dots-core/Cargo.toml"),
    "ui/package.json": JSON.parse(read("ui/package.json")).version,
    "Cargo.lock (dots-app)": locked("dots-app"),
    "Cargo.lock (dots-core)": locked("dots-core"),
  };
}

/// The public key of the prototype's throwaway signing key (minisign B6502CEA5C87EB8A),
/// which the plan left in tauri.conf.json until the owner's replaces it. A release signed
/// with the owner's key would reach no copy that carries this one.
export const PROTOTYPE_PUBKEY =
  "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEI2NTAyQ0VBNUM4N0VCOEEKUldTSzY0ZGM2aXhRdHUxQ0RJcGRmRGZTeVlqcUZJZzIzaEQ2NGxYOHNPUlpwL1lWdCtkTXVFY2MK";

/// Why the updater key in tauri.conf.json cannot be released with, or nothing.
function pubkeyError(root) {
  const where = "apps/dots-app/tauri.conf.json";
  const conf = JSON.parse(readFileSync(join(root, where), "utf8"));
  const pubkey = conf.plugins?.updater?.pubkey;
  if (!pubkey) return `${where} has no plugins.updater.pubkey (docs/releasing.md)`;
  if (pubkey.trim() === PROTOTYPE_PUBKEY) {
    return (
      `${where} has the prototype's updater key; put the owner's public key in ` +
      "plugins.updater.pubkey first (docs/releasing.md)"
    );
  }
  return null;
}

/// The tag's version and whether it is a prerelease, or the reasons the tag cannot be
/// released. Only a plain `vMAJOR.MINOR.PATCH` tag is a release; any other is a prerelease,
/// which GitHub's "latest" skips. No tag is released with the prototype's updater key.
export function checkTag(root, tag) {
  if (!/^v\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(tag)) {
    return { errors: [`the tag ${tag} is not v<version>, such as v0.2.0 or v0.2.0-rc.1`] };
  }
  const version = tag.slice(1);
  const errors = Object.entries(versionFields(root))
    .filter(([, found]) => found !== version)
    .map(([where, found]) => `${where} has ${found ?? "no version"}, the tag ${tag}`);
  const key = pubkeyError(root);
  if (key) errors.push(key);
  return { version, prerelease: version.includes("-"), errors };
}

/// The updater manifest for one Windows installer. `notes`: the release's notes
/// (`release-notes.md`), when the tag has them.
export function manifest({ version, signature, installer, date, notes }) {
  return {
    version,
    notes: notes?.trim() || `Dots ${version}`,
    pub_date: date.toISOString(),
    platforms: {
      [PLATFORM]: {
        signature: signature.trim(),
        url: `${DOWNLOADS}/v${version}/${encodeURIComponent(basename(installer))}`,
      },
    },
  };
}

function main([command, tag, installer, out, notesFile]) {
  const root = fileURLToPath(new URL("../..", import.meta.url));
  const checked = tag ? checkTag(root, tag) : { errors: ["no tag given"] };
  if (checked.errors.length > 0) {
    for (const error of checked.errors) console.error(`release: ${error}`);
    process.exit(1);
  }
  if (command === "check") {
    const line = `prerelease=${checked.prerelease}`;
    console.log(line);
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${line}\n`);
  } else if (command === "manifest" && installer && out) {
    const signature = readFileSync(`${installer}.sig`, "utf8");
    const notes = notesFile && existsSync(notesFile) ? readFileSync(notesFile, "utf8") : undefined;
    const body = manifest({
      version: checked.version,
      signature,
      installer,
      date: new Date(),
      notes,
    });
    writeFileSync(out, `${JSON.stringify(body, null, 2)}\n`);
    console.log(`release: wrote ${out}`);
  } else {
    console.error("usage: release.mjs check <tag> | manifest <tag> <installer> <out> [notes-file]");
    process.exit(2);
  }
}

/// Whether `script`, the path node was started with, is this file. Both sides are resolved,
/// and on Windows compared without case: a drive letter or a folder's case may differ there,
/// and a check that did not run must not pass for one that did.
export function isThisFile(script, platform = process.platform) {
  if (!script) return false;
  const norm = (path) => (platform === "win32" ? resolve(path).toLowerCase() : resolve(path));
  return norm(fileURLToPath(import.meta.url)) === norm(script);
}

if (isThisFile(process.argv[1])) {
  main(process.argv.slice(2));
}
