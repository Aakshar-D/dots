# Voice tooling

`sherpa-lib.mjs` fetches the prebuilt sherpa-onnx library that `crates/dots-voice` links, and
checks it against a size and SHA-256 pinned in the script. The `sherpa-onnx-sys` build script
downloads that archive from GitHub without any check, so release and CI builds fetch it with
this script first and point the build script at the verified file.

    node tools/voice/sherpa-lib.mjs unpack --target <triple> --out <dir>
    export SHERPA_ONNX_LIB_DIR=<dir>/unpacked/<triple>/<archive>/lib   # the line it prints

CI and release builds use `unpack`: it fetches and verifies the archive, unpacks it under
`<dir>/unpacked` (with `tar`) and prints `SHERPA_ONNX_LIB_DIR`. `sherpa-onnx-sys` 2.0.1's build
script (`resolve_lib_dir`) links a folder given in that variable as it is: it downloads
nothing and keeps nothing in the build's target folder. That matters for a cached build: the
build script runs again only when the variable changes, so a link path it once wrote into
`target/sherpa-onnx-prebuilt` would outlive a cache that dropped that folder, and the link
would fail. CI unpacks to `$RUNNER_TEMP` on every run, outside `target`.

A Mac universal build compiles both architectures with the one variable, so `unpack` with
`--target aarch64-apple-darwin --target x86_64-apple-darwin` merges the two folders with
`lipo -create` into `<dir>/unpacked/universal/lib`; the linker takes each architecture's half.

`fetch` (the archive only, and `SHERPA_ONNX_ARCHIVE_DIR=<dir>`, which the build script copies
into its own cache instead of downloading) is kept for local use; `--target-dir` checks that
cache for archives that are not pinned. Local builds need none of this: without either
variable the crate downloads as before.

## Local Windows builds

The Windows archives are built against the static C runtime (`static-MT`), so a Windows build
links everything else statically as well; otherwise a binary that uses the library gets
`LNK4098: defaultlib 'LIBCMT' conflicts with use of other libs`. CI sets
`RUSTFLAGS=-C target-feature=+crt-static` on its Windows jobs. Locally, put these two lines in
`.cargo/config.toml` (or set the same `RUSTFLAGS`):

    [target.'cfg(all(windows, target_env = "msvc"))']
    rustflags = ["-C", "target-feature=+crt-static"]

Changing the flag rebuilds everything once, so a separate target directory keeps the old
build. Without it, debug builds still work on the machines tried so far, with the warning.

## Refreshing the pins

To refresh the pins when the `sherpa-onnx` crate version changes: update `VERSION` and every
`file` in `ARCHIVES` (the names come from `archive_name` in the new `sherpa-onnx-sys` build.rs,
static mode), download each archive from the release page, and record its byte size and
`sha256sum`. Run `node --test tools/voice/sherpa-lib.test.mjs`.
