# Project graph implementation plan

**Goal:** a per-project code graph built by the core with tree-sitter, kept up to date incrementally, offered to runs as the `graph` tool and a prompt map, and shown on a Graph screen.

**Spec:** `docs/superpowers/specs/2026-11-10-project-graph-design.md`.

**Architecture:** a new core module `graph` (`project`, `extract` per language, `store` over rusqlite, `link`, `community`, `map`, `query`, `export`, and the `Graphs` service). The runner gets the service (`set_graphs`), offers the tool through `RunTools::with_graph`, appends the prompt section and asks for a rebuild when a run ends. Both engines serve the tool. The app adds seven commands; the UI adds a lazy Graph screen.

**New dependencies:** `tree-sitter` and the grammars `tree-sitter-rust`, `-typescript`, `-javascript`, `-python`, `-go` (core); `cytoscape` (UI, in the Graph screen's own chunk).

## Tasks

1. Spec and plan (this file).
2. `graph::project`: root resolution and the key; tests with a repo, a linked worktree, a subfolder and a plain folder.
3. `graph::extract`: facts per language from tree-sitter, Markdown by lines; one fixture test per language.
4. `graph::store`: the per-project SQLite file, files with facts, nodes, edges, meta.
5. `graph::link` and `graph::community`: resolution of imports and calls, confidence, Louvain; tests on fixtures.
6. `graph::build`: scan with `ignore`, the size and time check, parse changed files, link, write; incremental tests (change, add, delete).
7. `graph::map`: the map, deterministic and capped; tests.
8. `graph::query`: the tool's actions with caps; tests.
9. `graph::export`: graphify's format; test.
10. `Graphs` service: one build per project, dirty flag, progress events (`RuntimeEvent::GraphUpdated`), loaded graph cache.
11. Wiring: `DotSpec::use_graph` with migration `0045`; policy `Graph`; the local engine's tool; the `dots` MCP server's tool; the Claude args allow; the runner (offer, prompt, first build, rebuild after a run); the runtime's methods for the app.
12. App commands, contract samples, fixture.
13. UI: types, commands, events, the Graph screen (lazy route), the Graph button, the dot editor's switch; tests.
14. Gates, the smoke on this repository, the PR.
