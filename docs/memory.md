# Memory

Sage remembers things across runs in two places.

- **A sol's memory** belongs to one sol. It is kept in Sage's database.
- **A project's memory** belongs to the project a sol works in. Every sol that works in that project reads and writes it. It is kept as files in the project's folder.

The design is in `docs/superpowers/specs/2026-11-19-memory-design.md`.

## What a project is

The project is the folder a sol works in. If that folder is inside a git repository, the project is the repository's main checkout, so a sol that works in a subfolder or on its own branch shares the same memory.

A sol whose folder is missing has no project memory.

## Four kinds

| Kind | Use it for | Limit |
|---|---|---|
| core | A basic that every run needs: what the project is, a standing preference | 160 characters |
| semantic | A fact or a decision | 500 characters |
| procedural | Steps that worked, or a pitfall | 500 characters |
| episodic | What one run did, on one dated line | 500 characters |

A sol keeps up to 200 memories. Sage writes up to 300 for a project.

## The files

A project's memory is in `<project folder>/.sage/memory/`. Each fact is one small Markdown file:

```
---
id: 01JD3K8M2Q4T6V8X0Z2B4D6F8H
kind: procedural
created: 2026-11-19T10:04:11.512Z
updated: 2026-11-19T10:04:11.512Z
source: sol
pinned: false
files:
  - ui/package.json
---
Run the UI tests with bun x vitest run, from the ui folder.
```

`MEMORY.md` in the same folder lists every fact on one line. Sage rewrites it; do not edit it.

You can edit the fact files by hand:

- **Change a fact**: edit the text under the second `---`.
- **Delete a fact**: delete its file.
- **Add a fact**: add a `.md` file with the text in it. The part between the `---` lines is optional. Without it the fact is semantic and written by you.

Sage looks at the folder each time it uses the memory and reads the files again when one was added, changed or deleted, so there is nothing to refresh.

If you copy a fact's file to start a new fact, the copy gets an id of its own; the original keeps its id.

Sage skips a file it cannot read as a fact: one that is empty, larger than 16 KB, not text, or that looks like it holds a secret. The Memory tab says how many it skipped.

### Git

Sage does not change `.gitignore` and does not commit anything. Choose one:

- Commit `.sage/memory/` to share the memory with everyone who clones the project.
- Add `.sage/` to `.gitignore` to keep the memory on this computer.

## What a run sees

A run's first prompt has a short section, "What you remember", of at most 2400 bytes:

- the core memories in full (up to 600 bytes);
- the titles of other memories (up to 1500 bytes), each with its id.

The sol reads a fact's full text only when it calls `recall`. Sage never stores a transcript as memory and never reads one to build memory.

## The tools

A sol has three tools, under the one permission **Memory**:

- `remember` saves a fact. It can say the kind, whether the fact is the project's or the sol's own, and which project files it is about.
- `recall` searches the sol's memory and its project's, or reads facts by id.
- `forget` deletes a fact. A sol cannot delete a fact you wrote.

Without a scope, a fact that names project files goes to the project. Any other fact goes to the sol.

Set **Memory** to deny in a sol's permissions to turn its memory off: no section in the prompt, no tools.

## Secrets

Sage refuses a memory that looks like a password, a key or a token, whoever writes it. Keep secrets in the vault.
