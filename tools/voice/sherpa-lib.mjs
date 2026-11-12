// Fetches the prebuilt sherpa-onnx library archive for a build target and checks it against the
// size and SHA-256 pinned below, so that release and CI builds link only a library this
// repository has vouched for. The sherpa-onnx-sys build script downloads its archive without
// any check; given SHERPA_ONNX_ARCHIVE_DIR it copies the archive of that folder instead and
// never downloads (`node --test tools/voice/sherpa-lib.test.mjs`).
//
//   node tools/voice/sherpa-lib.mjs fetch --target <triple> [--target <triple>...] --out <dir>
//                                        [--target-dir <dir>]
//       Downloads each target's archive into <dir> under its exact file name, writing it only
//       once its size and hash are right; a wrong one is reported and the exit code is 1. An
//       archive already in <dir> with the right hash is left alone. Prints the line to export
//       (SHERPA_ONNX_ARCHIVE_DIR=<dir>) on stdout; everything else goes to stderr, so
//       `... >> "$GITHUB_ENV"` is safe. With --target-dir, the build script's own cache in
//       <dir>/sherpa-onnx-prebuilt is checked too (see guardCache).
//
//   node tools/voice/sherpa-lib.mjs unpack --target <triple> [--target <triple>...] --out <dir>
//       Fetches as `fetch` does, then unpacks each verified archive under <dir>/unpacked and
//       prints SHERPA_ONNX_LIB_DIR=<its lib folder>. The build script then links that folder
//       directly and keeps nothing of its own in the build's target folder, so a restored
//       build cache can never point the link at a folder that is gone. Two Mac targets (a
//       universal build, one variable for both) are merged with `lipo` into one folder of
//       universal libraries, from which the linker takes each architecture's half.
//
// The pins are for sherpa-onnx-sys 2.0.1 in static link mode, the archive its build script
// picks for each target. See tools/voice/README.md to refresh them.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/// The release the archives are published under; the version of the sherpa-onnx-sys crate.
const VERSION = "2.0.1";
const DOWNLOADS = `https://github.com/k2-fsa/sherpa-onnx/releases/download/v${VERSION}`;
/// How long a download may take before it is given up.
const TIMEOUT_MS = 15 * 60 * 1000;

/// The archive of each build target: its file name, as the build script spells it, its size in
/// bytes and its SHA-256.
export const ARCHIVES = {
  "x86_64-pc-windows-msvc": {
    file: `sherpa-onnx-v${VERSION}-win-x64-static-MT-Release-lib.tar.bz2`,
    size: 122863596,
    sha256: "23e7b986b197eda0bbc47ff3d7d5c87afd6ef2b9077214ba26b0f1ec52ef0e66",
  },
  "aarch64-pc-windows-msvc": {
    file: `sherpa-onnx-v${VERSION}-win-arm64-static-MT-Release-lib.tar.bz2`,
    size: 140877151,
    sha256: "ef118817dcf0a585974159855ff8b2a0f4c8069a158766dfd92a5f702b0173be",
  },
  "aarch64-apple-darwin": {
    file: `sherpa-onnx-v${VERSION}-osx-arm64-static-lib.tar.bz2`,
    size: 20829454,
    sha256: "d7a9b14f337958e7373c8c650f046df33d210bbaff2f33cb8322dda663ee5e1c",
  },
  "x86_64-apple-darwin": {
    file: `sherpa-onnx-v${VERSION}-osx-x64-static-lib.tar.bz2`,
    size: 20415061,
    sha256: "6a3cb3c14104d3a02e7b2a6cd5621f590b14892cce7d0bc0935f7655ef518938",
  },
};

/// The pinned archive of a build target; throws for a target that has none.
export function archiveFor(target, table = ARCHIVES) {
  const entry = Object.hasOwn(table, target) ? table[target] : undefined;
  if (!entry) {
    throw new Error(
      `no sherpa-onnx archive is pinned for target '${target}' (known: ${Object.keys(table).join(", ")})`,
    );
  }
  return entry;
}

/// Whether `input`, bytes or the path of a file, has the size and SHA-256 of `expected`
/// ({ size, sha256 }). Gives { ok, size, sha256, problem }, where `problem` says what differs.
export function verify(input, expected) {
  const bytes = typeof input === "string" ? readFileSync(input) : input;
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  const size = bytes.length;
  let problem = null;
  if (size !== expected.size) problem = `size ${size}, expected ${expected.size}`;
  else if (sha256 !== expected.sha256.toLowerCase()) problem = `sha256 ${sha256}, expected ${expected.sha256}`;
  return { ok: problem === null, size, sha256, problem };
}

/// Downloads a URL into memory, following redirects, giving up after the timeout.
export async function download(url) {
  const response = await fetch(url, {
    redirect: "follow",
    headers: { "user-agent": "dots-sherpa-lib" },
    signal: AbortSignal.timeout(TIMEOUT_MS),
  });
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}

/// Puts the verified archive of `target` into `out`. Returns "cached" when it was already
/// there, "fetched" when it was downloaded. Throws, leaving no file of that name, when the
/// download does not match the pin. `download` and `table` are injectable for the tests.
export async function fetchTarget(target, out, { download: get = download, table = ARCHIVES, log = () => {} } = {}) {
  const entry = archiveFor(target, table);
  const path = join(out, entry.file);
  if (existsSync(path)) {
    if (verify(path, entry).ok) {
      log(`sherpa-lib: ${entry.file} is already in ${out} and verified`);
      return "cached";
    }
    rmSync(path, { force: true });
  }
  const url = `${DOWNLOADS}/${entry.file}`;
  log(`sherpa-lib: downloading ${url}`);
  const bytes = await get(url);
  const result = verify(bytes, entry);
  if (!result.ok) {
    throw new Error(
      `${entry.file} does not match its pin: expected sha256 ${entry.sha256} (${entry.size} bytes), ` +
        `got ${result.sha256} (${result.size} bytes)`,
    );
  }
  mkdirSync(out, { recursive: true });
  const part = `${path}.part`;
  writeFileSync(part, bytes);
  renameSync(part, path);
  log(`sherpa-lib: ${entry.file} verified (${entry.size} bytes, sha256 ${entry.sha256})`);
  return "fetched";
}

/// The build script ignores SHERPA_ONNX_ARCHIVE_DIR when its own cache,
/// <target-dir>/sherpa-onnx-prebuilt, already has the archive, or the unpacked library of one,
/// and a restored build cache can hold what no one verified. Returns the problems found: an
/// archive there that is not pinned or whose hash differs, and an unpacked folder without a
/// pinned archive beside it. Nothing is wrong when there is no such cache (a clean build).
export function guardCache(targetDir, table = ARCHIVES) {
  const cache = join(targetDir, "sherpa-onnx-prebuilt");
  if (!existsSync(cache)) return [];
  const pinned = new Map(Object.values(table).map((entry) => [entry.file, entry]));
  const problems = [];
  const verified = new Set();
  for (const name of readdirSync(cache)) {
    const path = join(cache, name);
    if (!statSync(path).isFile()) continue;
    const entry = pinned.get(name);
    if (!entry) problems.push(`${path} is not a pinned archive`);
    else if (!verify(path, entry).ok) problems.push(`${path} does not match its pin`);
    else verified.add(name);
  }
  for (const name of readdirSync(cache)) {
    if (!statSync(join(cache, name)).isDirectory()) continue;
    if (!verified.has(`${name}.tar.bz2`)) {
      problems.push(`${join(cache, name)} was unpacked from an archive that is not verified`);
    }
  }
  return problems;
}

/// Runs a program and throws with its output when it fails.
function runProgram(program, args, cwd) {
  const result = spawnSync(program, args, { cwd, encoding: "utf8" });
  if (result.error) throw new Error(`${program}: ${result.error.message}`);
  if (result.status !== 0) {
    throw new Error(`${program} ${args.join(" ")} failed (${result.status}): ${result.stderr}`);
  }
}

/// The folder `unpack` leaves the libraries of `target` in, under `out`.
export function libDirOf(out, target, table = ARCHIVES) {
  const stem = archiveFor(target, table).file.replace(/\.tar\.bz2$/, "");
  return join(out, "unpacked", target, stem, "lib");
}

/// Unpacks the verified archive of `target` (already in `out`) under `out/unpacked/<target>`,
/// replacing what an earlier run left; returns its lib folder. `tar` is given paths relative
/// to `out`, because GNU tar reads "D:" in a Windows path as a host name.
export function unpackTarget(target, out, { run = runProgram, table = ARCHIVES } = {}) {
  const entry = archiveFor(target, table);
  // Forward slashes: MSYS's GNU tar on Windows does not take a backslash path.
  const dest = `unpacked/${target}`;
  rmSync(join(out, dest), { recursive: true, force: true });
  mkdirSync(join(out, dest), { recursive: true });
  run("tar", ["-xjf", entry.file, "-C", dest], out);
  const lib = libDirOf(out, target, table);
  if (!existsSync(lib)) throw new Error(`${entry.file} held no lib folder (looked for ${lib})`);
  return lib;
}

/// Makes `into` hold, for every static library (`.a`) the first folder of `dirs` has, one
/// universal library of all their copies (`lipo -create`, macOS only).
export function mergeLibs(dirs, into, { run = runProgram } = {}) {
  rmSync(into, { recursive: true, force: true });
  mkdirSync(into, { recursive: true });
  const names = readdirSync(dirs[0]).filter((name) => name.endsWith(".a"));
  if (names.length === 0) throw new Error(`no static libraries in ${dirs[0]}`);
  for (const name of names) {
    const copies = dirs.map((dir) => join(dir, name));
    for (const copy of copies) {
      if (!existsSync(copy)) throw new Error(`${copy} is missing: the archives do not match`);
    }
    run("lipo", ["-create", "-output", join(into, name), ...copies]);
  }
  return into;
}

/// Fetches, verifies and unpacks the archives of `targets` under `out`; the folder for
/// SHERPA_ONNX_LIB_DIR. Several targets must all be Mac ones: they are merged.
export async function unpack(targets, out, { download: get, table = ARCHIVES, run = runProgram, log = () => {} } = {}) {
  if (targets.length > 1 && !targets.every((t) => t.endsWith("-apple-darwin"))) {
    throw new Error("only Mac targets can share one library folder (a universal build)");
  }
  const libs = [];
  for (const target of targets) {
    await fetchTarget(target, out, { download: get, table, log });
    libs.push(unpackTarget(target, out, { run, table }));
    log(`sherpa-lib: ${target} unpacked in ${libs.at(-1)}`);
  }
  if (libs.length === 1) return libs[0];
  const universal = mergeLibs(libs, join(out, "unpacked", "universal", "lib"), { run });
  log(`sherpa-lib: universal libraries in ${universal}`);
  return universal;
}

/// The options of the command line, `--target` as many times as given.
export function parseArgs(args) {
  const options = { targets: [], out: null, targetDir: null };
  for (let i = 0; i < args.length; i += 2) {
    const value = args[i + 1];
    if (value === undefined) throw new Error(`${args[i]} needs a value`);
    if (args[i] === "--target") options.targets.push(value);
    else if (args[i] === "--out") options.out = value;
    else if (args[i] === "--target-dir") options.targetDir = value;
    else throw new Error(`unknown option ${args[i]}`);
  }
  return options;
}

async function main(args) {
  const log = (line) => console.error(line);
  try {
    if (args[0] !== "fetch" && args[0] !== "unpack") {
      throw new Error("usage: sherpa-lib.mjs fetch|unpack --target <triple> [--target <triple>...] --out <dir> [--target-dir <dir>]");
    }
    const options = parseArgs(args.slice(1));
    if (options.targets.length === 0 || !options.out) throw new Error(`${args[0]} needs --target and --out`);
    const out = resolve(options.out);
    if (args[0] === "unpack") {
      const lib = await unpack(options.targets, out, { log });
      console.log(`SHERPA_ONNX_LIB_DIR=${lib}`);
      return;
    }
    for (const target of options.targets) await fetchTarget(target, out, { log });
    if (options.targetDir) {
      const problems = guardCache(resolve(options.targetDir));
      if (problems.length > 0) {
        for (const problem of problems) log(`sherpa-lib: ${problem}`);
        log("sherpa-lib: delete the build's sherpa-onnx-prebuilt folder, or the build cache, and run again");
        process.exit(1);
      }
    }
    console.log(`SHERPA_ONNX_ARCHIVE_DIR=${out}`);
  } catch (error) {
    log(`sherpa-lib: ${error.message}`);
    process.exit(1);
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url).toLowerCase() === resolve(process.argv[1]).toLowerCase()) {
  main(process.argv.slice(2));
}
