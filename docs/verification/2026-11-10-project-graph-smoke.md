# Project graph smoke (2026-11-10)

Run by the builder on the Windows ARM64 PC, in the worktree `C:/dev/dots-project-graph`, at the branch's head.

## 1. This repository's graph

`GRAPH_SMOKE_ROOT=C:/dev/dots-project-graph GRAPH_SMOKE_TOUCH=crates/dots-core/src/runner.rs cargo test --release -p dots-core --test project_graph_test smoke_build -- --ignored --nocapture`

It builds into a fresh graph file in a temporary folder, builds again with nothing changed, then again after appending a line to `runner.rs` (put back afterwards).

| build | files | parsed | nodes | edges | time |
|---|---|---|---|---|---|
| first | 1,041 (6 skipped) | 1,041 | 15,212 | 48,513 | 1.1 s |
| nothing changed | 1,041 | 0 | 15,212 | 48,513 | 16 ms |
| one file changed | 1,041 | 1 | 15,212 | 48,513 | 175 ms |

- Languages: Rust 511, TSX 273, Markdown 129, TypeScript 107, JavaScript 21.
- 15 communities. The prompt's map is 4,106 bytes (about 1k tokens); the tool's `map` is 6,073 bytes.
- Release build. While other agents were building on the same PC the first build took 17.8 s and the one-file rebuild 0.7 s. A debug build is about ten times slower (84 s first build).
- The skipped six are files over 512 KB (fixtures and lock files).

The map's top lines:

```
# Project map
1041 files, 15212 nodes, 33379 links. Languages: rust 511, tsx 273, markdown 129, typescript 107, javascript 21.

## Layout
- crates/: 501 files (dots-core 491, dots-admin 10)
- ui/: 381 files (src 379, public 1)
- docs/: 126 files (superpowers 95, verification 26)
...
## Entry points
- apps/dots-app/src/commands.rs: 281 Tauri commands
- apps/dots-app/src/main.rs: entry file, fn main
...
## Hubs (most linked)
- temp_store (function) crates/dots-core/tests/common/mod.rs:25, 396 links
- NewRun.new (method) crates/dots-core/src/model.rs:452, 250 links
- Store.create_run (method) crates/dots-core/src/store/runs.rs:286, 166 links
...
```

What it shows: tests weigh heavily in this repository's clusters and hubs (the test helpers are the most called functions). That is true of the code, but a map that left tests out of the hubs would say more about the product. Left for later.

## 2. The real `claude` CLI asks the graph

`CLAUDE_SMOKE=<path to claude.exe> cargo test -p dots-core --test project_graph_test smoke_the_real_cli -- --ignored --nocapture`

A Claude dot (haiku, 4 turns, Sandboxed) in a two-function repository with a built graph, told to call `mcp__dots__graph` with `callers` of `helper` and answer with the node id.

It passed in 9 s. The run's events:

```
tool_call: {"input":{"action":"callers","node":"helper"},"tool":"mcp__dots__graph"}
tool_result: {"is_error":false,"output":"src/lib.rs#helper is called by 1:\n- src/lib.rs#start :1 (function) (call at line 1)"}
assistant_text: "src/lib.rs#helper"
status Succeeded
```

So the CLI lists the tool, Dots' `dots` server checks the call as `Graph` (allowed by Sandboxed without asking), and the answer comes from the graph.

## 3. Sizes

- UI: the main bundle grew by 2.3 KB (1,363,167 to 1,365,433 bytes). The Graph screen is its own chunk, loaded when first opened: 444 KB (141 KB gzipped), almost all Cytoscape.js.
- App binary (release `dots-app.exe`, ARM64): 54,205,952 bytes on master (998ca64b), 60,623,360 bytes on this branch: +6.4 MB (+12%). Most of it is the grammars' parse tables (TypeScript and TSX are the largest).
- Compile time: the six tree-sitter crates (C, built with clang) take 33 s alone in a fresh release target folder with 6 jobs; in a full build they compile alongside the Rust crates. A clean release build of the app took 597 s on master; the incremental release build after this branch's changes took 442 s (the PC was shared with other builds, so these are rough).

## 4. Left for the lead's QA in the app

- The Graph screen on a real dot: the canvas, cluster view on a large repository, Show in folder, Export for graphify (writes `<repo>/graphify-out/graph.json`; an existing one not from Dots is kept as `graph.json.bak-<milliseconds>[-n]`).
- The Use project graph switch in the dot editor.
- A Claude run's first prompt carrying the map, and a local-model run calling `graph`.
