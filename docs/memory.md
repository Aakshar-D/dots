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

The titles are chosen by what the run is about: the facts that match the words of the message come first, and so do facts about a file the message names. The rest follow, pinned first, then newest.

The sol reads a fact's full text only when it calls `recall`. Sage never stores a transcript as memory and never reads one to build memory.

## Facts about files

A fact can name the project files it is about (`files:` in its file). Then:

- `recall` with a file gives the facts about that file, about a folder above it, or about files below it.
- A run whose message names the file gets those facts among its titles.
- When a sol asks the project graph about a file, the answer ends with how many facts are about it.

## The tools

A sol has three tools, under the one permission **Memory**:

- `remember` saves a fact. It can say the kind, whether the fact is the project's or the sol's own, and which project files it is about.
- `recall` searches the sol's memory and its project's. The best matches come first, at most 20. It can also give the facts about one project file, or read facts by id.
- `forget` deletes a fact. A sol cannot delete a fact you wrote.

Without a scope, a fact that names project files goes to the project. Any other fact goes to the sol.

Set **Memory** to deny in a sol's permissions to turn its memory off: no section in the prompt, no tools.

## Saved after each run

When a run that did some work ends, Sage saves what it learned without the sol having to ask:

- one dated line about the run, taken from the first line of its answer that says something (a heading or a bare "Done." is passed over);
- up to three facts, picked by one small request to the cheapest model of the sol's engine.

They go to the project's memory when the sol has a project, otherwise to the sol's own.

The request is made after the run has ended, so it never holds a run. It reads the run's final answer and a short list of what the run did (which tools, which files). It never reads the conversation. Its usage counts toward the run.

A run is skipped when it failed, when its answer is shorter than 200 characters, when it called no tool, or when the sol's **Memory** permission is not allow.

A fact the memory has already is not saved again. A fact that corrects an older one replaces it.

Turn this off in **Settings > Runs & server > Memory > Save memories after each run**.

## Keeping it small

A sol or a project keeps the newest 50 dated lines. When there is no room, the oldest line that no sol ever recalled goes first. Facts you wrote, and anything pinned, are never removed this way.

## The Memory tab

Open a sol's editor and choose **Memory**. It shows two lists:

- **This sol**: what the sol itself remembers.
- **The project**: what the project remembers, with the path of the memory folder and **Open folder**.

Each list is grouped by kind. You can add, edit, pin and delete in both. Choose the kind in the box when you add or edit.

## Secrets

Sage refuses a memory that looks like a password, a key or a token, whoever writes it. Keep secrets in the vault.
