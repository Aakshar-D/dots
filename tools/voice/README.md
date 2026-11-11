# Voice tooling

`sherpa-lib.mjs` fetches the prebuilt sherpa-onnx library that `crates/dots-voice` links, and
checks it against a size and SHA-256 pinned in the script. The `sherpa-onnx-sys` build script
downloads that archive from GitHub without any check, so release and CI builds fetch it with
this script first and point the build script at the verified file.

    node tools/voice/sherpa-lib.mjs fetch --target <triple> --out <dir>
    export SHERPA_ONNX_ARCHIVE_DIR=<dir>      # the line the script prints

With that variable the build script copies `<dir>/<archive>` and never downloads; a folder
without the archive fails the build rather than falling back to a download. It does skip the
variable when its own cache (`<target dir>/sherpa-onnx-prebuilt`) already holds the archive or
an unpacked copy, so CI passes `--target-dir target` and the script fails on a cache entry that
is not pinned. Local builds need none of this: without the variable the crate downloads as before.

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
