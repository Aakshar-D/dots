//! Ranked recall and file links (memory spec section 7): `recall` answers the best matches
//! first from a full-text index that is only a cache, the prompt's titles are chosen by what
//! the run is about, a memory is found by the project file it is linked to, and none of it
//! fails a run when the index cannot be used.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{spec, temp_store};
use dots_core::events::new_bus;
use dots_core::graph::{GraphState, Graphs, Project, Scope};
use dots_core::memory::files;
use dots_core::memory::{
    prompt_section, Memories, NewFact, Tool, NOTHING, RECALL_BUDGET, RECALL_LIMIT,
};
use dots_core::model::{
    Dot, Memory, MemoryKind, MemoryScope, MemorySource, NewRun, TriggerKind, WorkspaceMode,
};
use dots_core::run_tools::RunGraph;
use dots_core::store::Store;
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use tempfile::TempDir;

struct Setup {
    dir: TempDir,
    store: Store,
    memories: Memories,
}

async fn setup() -> Setup {
    let (dir, store) = temp_store().await;
    let memories = Memories::new(store.clone(), new_bus());
    Setup {
        dir,
        store,
        memories,
    }
}

impl Setup {
    fn folder(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    async fn dot(&self, name: &str, folder: &Path) -> Dot {
        let mut s = spec(name);
        s.workdir = folder.to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        self.store.create_dot(&s, None, "default").await.unwrap()
    }

    async fn run(&self, dot: &Dot) -> String {
        self.store
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap()
            .id
    }

    /// A memory of the project at `root`, about `files`.
    async fn project_fact(&self, root: &Path, text: &str, files: &[&str]) -> Memory {
        let files: Vec<String> = files.iter().map(|f| f.to_string()).collect();
        let new = NewFact {
            files: &files,
            ..NewFact::plain(text, MemorySource::Dot, None)
        };
        let saved = self.memories.project_save(root, new).await.unwrap();
        tick().await;
        saved
    }

    async fn sol_fact(&self, dot: &Dot, text: &str) -> Memory {
        let saved = self
            .memories
            .save(&dot.id, text, MemorySource::Dot, None)
            .await
            .unwrap();
        tick().await;
        saved
    }

    async fn recall(&self, run: &str, input: Value) -> String {
        self.memories.call(run, Tool::Recall, &input).await.unwrap()
    }

    /// A second connection to the test's database, to change the index behind the store.
    async fn raw(&self) -> SqlitePool {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(self.dir.path().join("dots.db")))
            .await
            .unwrap()
    }
}

/// Times are stored to the millisecond: memories saved one after another need a tick between.
async fn tick() {
    tokio::time::sleep(Duration::from_millis(3)).await;
}

/// The ids of the memories `answer` shows, in its order.
fn ids_of(answer: &str) -> Vec<String> {
    answer
        .lines()
        .filter_map(|l| {
            l.strip_prefix('[')?
                .split_once(']')
                .map(|(id, _)| id.to_string())
        })
        .collect()
}

#[tokio::test]
async fn recall_answers_the_best_matches_first_across_both_scopes() {
    let t = setup().await;
    let root = t.folder("shop");
    let dot = t.dot("builder", &root).await;
    let run = t.run(&dot).await;
    for i in 0..12 {
        t.project_fact(
            &root,
            &format!("Filler note {i} about nothing in particular."),
            &[],
        )
        .await;
    }
    let staging = t
        .sol_fact(&dot, "The staging server is staging.example.com.")
        .await;
    let deploys = t
        .project_fact(
            &root,
            "Deploys go out on Fridays after the staging check.",
            &[],
        )
        .await;
    let pnpm = t
        .project_fact(&root, "Use pnpm, never npm, to install packages.", &[])
        .await;
    let weekly = t.sol_fact(&dot, "Backups run weekly on Sundays.").await;
    let nightly = t.sol_fact(&dot, "Backups run nightly at two.").await;

    // Any word counts, the memory with more of them first, whichever scope it is in.
    let found = t.recall(&run, json!({"query": "staging deploy"})).await;
    assert_eq!(
        ids_of(&found),
        [deploys.id.as_str(), staging.id.as_str()],
        "{found}"
    );
    assert!(
        found.starts_with(&format!("[{}] Deploys go out", deploys.id)),
        "{found}"
    );
    // A word's start is enough; case and marks do not matter.
    assert_eq!(
        ids_of(&t.recall(&run, json!({"query": "PNPM!"})).await),
        [pnpm.id.as_str()]
    );
    assert_eq!(
        ids_of(&t.recall(&run, json!({"query": "install packag"})).await),
        [pnpm.id.as_str()]
    );
    assert_eq!(
        t.recall(&run, json!({"query": "kubernetes"})).await,
        NOTHING
    );
    // Words too common to rank by are matched as they stand: every one of them.
    assert_eq!(
        ids_of(&t.recall(&run, json!({"query": "the is"})).await),
        [staging.id.as_str()]
    );

    // Equal matches: the newer one first, and a pin goes before that.
    let found = t.recall(&run, json!({"query": "backups run"})).await;
    assert_eq!(
        ids_of(&found),
        [nightly.id.as_str(), weekly.id.as_str()],
        "{found}"
    );
    t.memories
        .update(&weekly.id, None, Some(true))
        .await
        .unwrap();
    let found = t.recall(&run, json!({"query": "backups run"})).await;
    assert_eq!(
        ids_of(&found),
        [weekly.id.as_str(), nightly.id.as_str()],
        "{found}"
    );

    // An empty query lists pinned first, then newest, of both scopes, at most 20.
    let listed = ids_of(&t.recall(&run, json!({})).await);
    assert_eq!(listed.len(), 17);
    assert_eq!(
        listed[..3],
        [weekly.id.as_str(), nightly.id.as_str(), pnpm.id.as_str()]
    );
    let listed = t.recall(&run, json!({"query": "  "})).await;
    assert_eq!(ids_of(&listed).len(), 17);

    // A dot elsewhere finds none of this, and its own memory is found only by it.
    let elsewhere = t.folder("elsewhere");
    let stranger = t.dot("stranger", &elsewhere).await;
    let theirs = t.sol_fact(&stranger, "pnpm is not used here.").await;
    let s_run = t.run(&stranger).await;
    assert_eq!(
        ids_of(
            &t.recall(&s_run, json!({"query": "pnpm staging backups"}))
                .await
        ),
        [theirs.id.as_str()]
    );
    assert_eq!(
        ids_of(&t.recall(&run, json!({"query": "pnpm"})).await),
        [pnpm.id.as_str()]
    );
}

#[tokio::test]
async fn recall_finds_memories_by_the_project_file_they_are_about() {
    let t = setup().await;
    let root = t.folder("shop");
    let dot = t.dot("builder", &root).await;
    let run = t.run(&dot).await;
    let types = t
        .project_fact(
            &root,
            "Regenerate the contract after changing a type.",
            &["ui/src/api/types.ts", "apps/app/src/contract.rs"],
        )
        .await;
    let ui = t
        .project_fact(&root, "The UI is tested with vitest.", &["ui"])
        .await;
    let core = t
        .project_fact(
            &root,
            "The core crate has no network code.",
            &["crates/core"],
        )
        .await;
    let many = t
        .project_fact(
            &root,
            "These four move together.",
            &["a/one.rs", "a/two.rs", "a/three.rs", "a/four.rs"],
        )
        .await;
    t.project_fact(&root, "No file at all.", &[]).await;

    // The file itself, and a folder above it; newest first.
    let found = t
        .recall(&run, json!({"file": "ui\\src\\api\\types.ts"}))
        .await;
    assert_eq!(
        ids_of(&found),
        [ui.id.as_str(), types.id.as_str()],
        "{found}"
    );
    assert!(
        found.contains(&format!(
            "[{}] Regenerate the contract after changing a type. (files: ui/src/api/types.ts, \
             apps/app/src/contract.rs)",
            types.id
        )),
        "{found}"
    );
    // A folder: what is about it and what is about a file below it.
    assert_eq!(
        ids_of(&t.recall(&run, json!({"file": "ui/"})).await),
        [ui.id.as_str(), types.id.as_str()]
    );
    assert_eq!(
        t.recall(&run, json!({"file": "docs/readme.md"})).await,
        NOTHING
    );
    assert_eq!(
        t.recall(&run, json!({"file": "../../etc/passwd"})).await,
        NOTHING
    );
    // With words too: the file's memories, the best match first, and all of them when no
    // word matches.
    assert_eq!(
        ids_of(
            &t.recall(&run, json!({"file": "ui", "query": "contract"}))
                .await
        ),
        [types.id.as_str()]
    );
    assert_eq!(
        ids_of(
            &t.recall(
                &run,
                json!({"file": "crates/core/src/lib.rs", "query": "zebra"})
            )
            .await
        ),
        [core.id.as_str()]
    );
    // A query that names a file finds what is linked to it, though no word of the text does.
    // It comes before a memory that only shares the ending `rs`.
    let by_name = t
        .recall(&run, json!({"query": "what about three.rs"}))
        .await;
    assert_eq!(ids_of(&by_name)[0], many.id, "{by_name}");
    let first = by_name.lines().next().unwrap();
    assert!(
        first.ends_with("(files: a/one.rs, a/two.rs, a/three.rs and 1 more)"),
        "{by_name}"
    );
    // The words of a path are found as words.
    assert_eq!(
        ids_of(&t.recall(&run, json!({"query": "contract.rs"})).await)[0],
        types.id
    );

    // What a `graph` answer about a file ends with.
    assert_eq!(
        files::linked_note(&root, "ui/src/api/types.ts").unwrap(),
        format!(
            "2 memories are about this file: [{}] [{}]. recall with `file` reads them.",
            ui.id, types.id
        )
    );
    assert_eq!(
        files::linked_note(&root, "crates/core/src/lib.rs").unwrap(),
        format!(
            "1 memory is about this file: [{}]. recall with `file` reads it.",
            core.id
        )
    );
    assert_eq!(files::linked_note(&root, "docs/readme.md"), None);
}

#[tokio::test]
async fn the_prompts_titles_are_chosen_by_what_the_run_is_about() {
    let t = setup().await;
    let root = t.folder("shop");
    let dot = t.dot("builder", &root).await;
    // The two that matter are the oldest, so pinned first then newest would leave them out.
    let tax = t
        .project_fact(&root, "The checkout total must include tax.", &[])
        .await;
    let rounding = t
        .project_fact(
            &root,
            "Rounding happens once, at the end.",
            &["ui/src/checkout/Total.tsx"],
        )
        .await;
    let basics = t
        .memories
        .project_save(
            &root,
            NewFact {
                kind: MemoryKind::Core,
                ..NewFact::plain(
                    "The shop is built with Next.js.",
                    MemorySource::Person,
                    None,
                )
            },
        )
        .await
        .unwrap();
    for i in 0..40 {
        t.project_fact(
            &root,
            &format!("Filler note {i} about nothing in particular."),
            &[],
        )
        .await;
    }
    let own = t.sol_fact(&dot, "I answer in short sentences.").await;

    let about = "Fix the checkout total shown in Total.tsx";
    let ordered = t.memories.for_prompt(&dot, about).await;
    assert_eq!(ordered.len(), 44);
    let first: Vec<&str> = ordered[..2].iter().map(|m| m.id.as_str()).collect();
    assert!(
        first.contains(&tax.id.as_str()) && first.contains(&rounding.id.as_str()),
        "{first:?}"
    );
    // The rest follows as before: newest first.
    assert_eq!(ordered[2].id, own.id);
    let section = prompt_section(&ordered);
    assert!(section.len() <= dots_core::memory::PROMPT_BUDGET);
    let core_line = format!("- [{}] The shop is built with Next.js.\n", basics.id);
    let tax_line = format!("- [{}] The checkout total must include tax.\n", tax.id);
    let rounding_line = format!("- [{}] Rounding happens once, at the end.\n", rounding.id);
    // Core first, whatever the run is about; then the two that answer it.
    let at = |line: &str| {
        section
            .find(line)
            .unwrap_or_else(|| panic!("{line} in {section}"))
    };
    assert!(
        at(&core_line) < at(&tax_line).min(at(&rounding_line)),
        "{section}"
    );
    assert!(at(&tax_line).max(at(&rounding_line)) < section.find("Filler note").unwrap());
    assert!(section.contains("more not shown"), "{section}");

    // A run about something else does not get them: they are too old to fit.
    let other = prompt_section(&t.memories.for_prompt(&dot, "Write the release notes").await);
    assert!(other.contains(&core_line), "{other}");
    assert!(
        !other.contains(&tax_line) && !other.contains(&rounding_line),
        "{other}"
    );
    // A text with no word to rank by keeps the old order.
    let plain = t.memories.for_prompt(&dot, "").await;
    assert_eq!(plain[0].id, own.id);
    assert_eq!(plain[43].id, tax.id);
}

#[tokio::test]
async fn the_index_is_a_cache_of_the_memories_as_they_are_now() {
    let t = setup().await;
    let root = t.folder("shop");
    let dot = t.dot("builder", &root).await;
    let run = t.run(&dot).await;
    let pnpm = t
        .project_fact(&root, "Use pnpm, never npm, to install packages.", &[])
        .await;
    let own = t.sol_fact(&dot, "The person likes pnpm.").await;
    let both = [own.id.as_str(), pnpm.id.as_str()];
    assert_eq!(
        ids_of(&t.recall(&run, json!({"query": "pnpm"})).await),
        both
    );

    // What recall answered with is noted in the index, and no file was written for it.
    let key = Project::resolve(&root).await.unwrap().key;
    let file = files::dir_of(&root).join(&files::load(&root).find(&pnpm.id).unwrap().file_name);
    let before = (
        std::fs::read_to_string(&file).unwrap(),
        std::fs::metadata(&file).unwrap().modified().unwrap(),
    );
    let recalled = t
        .store
        .recalled_memories(MemoryScope::Project, &key)
        .await
        .unwrap();
    assert!(
        recalled.contains(&pnpm.id) && recalled.len() == 1,
        "{recalled:?}"
    );
    let recalled = t
        .store
        .recalled_memories(MemoryScope::Dot, &dot.id)
        .await
        .unwrap();
    assert!(recalled.contains(&own.id), "{recalled:?}");
    tick().await;
    t.recall(&run, json!({"ids": [pnpm.id]})).await;
    let after = (
        std::fs::read_to_string(&file).unwrap(),
        std::fs::metadata(&file).unwrap().modified().unwrap(),
    );
    assert_eq!(before, after);

    // The index emptied behind the store: the next search fills it again.
    let raw = t.raw().await;
    for sql in ["DELETE FROM memory_fts", "DELETE FROM memory_docs"] {
        sqlx::query(sql).execute(&raw).await.unwrap();
    }
    assert_eq!(
        ids_of(&t.recall(&run, json!({"query": "pnpm"})).await),
        both
    );

    // A file edited by hand: the new words are found, the old ones are not.
    let edited = before
        .0
        .replace("Use pnpm, never npm,", "Use yarn, never bun,");
    std::fs::write(&file, edited).unwrap();
    assert_eq!(
        ids_of(&t.recall(&run, json!({"query": "yarn"})).await),
        [pnpm.id.as_str()]
    );
    assert_eq!(
        ids_of(&t.recall(&run, json!({"query": "pnpm"})).await),
        [own.id.as_str()]
    );
    // A file deleted by hand, and a memory deleted in the tab: gone from the answers.
    std::fs::remove_file(&file).unwrap();
    assert_eq!(t.recall(&run, json!({"query": "yarn"})).await, NOTHING);
    t.memories.delete(&own.id).await.unwrap();
    assert_eq!(t.recall(&run, json!({"query": "pnpm"})).await, NOTHING);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_docs")
        .fetch_one(&raw)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    // A deleted dot's rows leave the index with the next change.
    let again = t.sol_fact(&dot, "Remember the pnpm lockfile.").await;
    assert_eq!(
        ids_of(&t.recall(&run, json!({"query": "lockfile"})).await),
        [again.id.as_str()]
    );
    let other = t.dot("other", &t.folder("elsewhere")).await;
    let other_run = t.run(&other).await;
    t.sol_fact(&other, "Something of mine.").await;
    assert_eq!(
        t.recall(&other_run, json!({"query": "mine"}))
            .await
            .lines()
            .count(),
        1
    );
    t.store.delete_dot(&dot.id).await.unwrap();
    t.sol_fact(&other, "Something else of mine.").await;
    t.recall(&other_run, json!({"query": "mine"})).await;
    let owners: Vec<String> = sqlx::query_scalar("SELECT DISTINCT owner FROM memory_docs")
        .fetch_all(&raw)
        .await
        .unwrap();
    assert_eq!(owners, [other.id.as_str()]);
    let indexed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_fts")
        .fetch_one(&raw)
        .await
        .unwrap();
    assert_eq!(indexed, 2);
    raw.close().await;
}

#[tokio::test]
async fn a_broken_index_fails_nothing() {
    let t = setup().await;
    let root = t.folder("shop");
    let dot = t.dot("builder", &root).await;
    let run = t.run(&dot).await;
    let pnpm = t
        .project_fact(
            &root,
            "Use pnpm, never npm, to install packages.",
            &["package.json"],
        )
        .await;
    let deploys = t.sol_fact(&dot, "Deploys go out on Fridays.").await;
    for i in 0..30 {
        t.sol_fact(
            &dot,
            &format!("Filler note {i} about nothing in particular."),
        )
        .await;
    }
    let raw = t.raw().await;
    sqlx::query("DROP TABLE memory_fts")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;

    // recall counts the matching words itself.
    assert_eq!(
        ids_of(&t.recall(&run, json!({"query": "pnpm install"})).await),
        [pnpm.id.as_str()]
    );
    assert_eq!(
        ids_of(
            &t.recall(&run, json!({"query": "deploy fridays pnpm"}))
                .await
        ),
        [deploys.id.as_str(), pnpm.id.as_str()]
    );
    assert_eq!(
        t.recall(&run, json!({"query": "kubernetes"})).await,
        NOTHING
    );
    assert_eq!(
        ids_of(&t.recall(&run, json!({"file": "package.json"})).await),
        [pnpm.id.as_str()]
    );
    assert_eq!(ids_of(&t.recall(&run, json!({})).await).len(), RECALL_LIMIT);
    // And the prompt still puts what the run is about first.
    let ordered = t.memories.for_prompt(&dot, "why do deploys fail").await;
    assert_eq!(ordered.len(), 32);
    assert_eq!(ordered[0].id, deploys.id);
}

#[tokio::test]
async fn recalls_answer_is_bounded() {
    let t = setup().await;
    let root = t.folder("shop");
    let dot = t.dot("builder", &root).await;
    let run = t.run(&dot).await;
    for i in 0..30 {
        t.project_fact(&root, &format!("Alpha fact number {i}."), &[])
            .await;
    }
    // Twenty at most, and a line for the rest.
    let found = t.recall(&run, json!({"query": "alpha"})).await;
    assert_eq!(ids_of(&found).len(), RECALL_LIMIT);
    assert!(
        found.ends_with("\n(10 more: ask with more words, or read some by ids.)"),
        "{found}"
    );
    let listed = t.recall(&run, json!({})).await;
    assert_eq!(ids_of(&listed).len(), RECALL_LIMIT);

    // Long memories: fewer of them, within the answer's bytes.
    for i in 0..30 {
        let text = format!("Beta {i} {}", "long ".repeat(98));
        t.sol_fact(&dot, text.trim()).await;
    }
    let found = t.recall(&run, json!({"query": "beta"})).await;
    assert!(found.len() <= RECALL_BUDGET, "{}", found.len());
    let shown = ids_of(&found).len();
    assert!((8..RECALL_LIMIT).contains(&shown), "{shown}");
    assert!(
        found.ends_with(&format!(
            "\n({} more: ask with more words, or read some by ids.)",
            30 - shown
        )),
        "{found}"
    );
    // By ids: at most twenty, each in full.
    let all: Vec<String> = t
        .memories
        .readable(&dot)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    let by_ids = t.recall(&run, json!({"ids": all})).await;
    assert_eq!(by_ids.lines().count(), RECALL_LIMIT);
}

/// Memory spec section 7: a `graph` answer about a file ends with the project's memories that
/// are about it; a dot with memory off gets the answer alone.
#[tokio::test]
async fn a_graph_answer_about_a_file_names_the_memories_about_it() {
    let t = setup().await;
    let root = t.folder("proj");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn start() { helper(); }\nfn helper() {}\n",
    )
    .unwrap();
    let fact = t
        .project_fact(&root, "start() is the only entry point.", &["src/lib.rs"])
        .await;
    let graphs = Graphs::new(t.dir.path().join("graphs"), None);
    let project = Project::at_git(root.clone());
    graphs
        .call(&project, &Scope::all(), &json!({"action": "map"}))
        .await
        .unwrap();
    for _ in 0..500 {
        let s = graphs.status(&project);
        if s.state != GraphState::Building && s.built_at.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let graph = RunGraph {
        graphs,
        project,
        scope: Scope::all(),
        memory_root: Some(root.clone()),
    };
    let about_file = graph
        .call(&json!({"action": "file", "path": "src/lib.rs"}))
        .await
        .unwrap();
    assert!(about_file.contains("start"), "{about_file}");
    assert!(
        about_file.ends_with(&format!(
            "\n\n1 memory is about this file: [{}]. recall with `file` reads it.",
            fact.id
        )),
        "{about_file}"
    );
    // Only an answer about a file has the line.
    let map = graph.call(&json!({"action": "map"})).await.unwrap();
    assert!(!map.contains("memory is about"), "{map}");
    let off = RunGraph {
        memory_root: None,
        ..graph
    };
    let alone = off
        .call(&json!({"action": "file", "path": "src/lib.rs"}))
        .await
        .unwrap();
    assert!(!alone.contains("memory is about"), "{alone}");
}

/// Memory spec section 12: migration 0060 adds the index to a store that has memories
/// already, empty, and the first search fills it from them.
#[tokio::test]
async fn memories_stored_before_the_index_are_found_after_the_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let root = dir.path().join("shop");
    std::fs::create_dir_all(&root).unwrap();
    let (dot, own, fact) = {
        let store = Store::open(&path).await.unwrap();
        let memories = Memories::new(store.clone(), new_bus());
        let mut s = spec("builder");
        s.workdir = root.to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        let dot = store.create_dot(&s, None, "default").await.unwrap();
        let own = memories
            .save(&dot.id, "The person likes pnpm.", MemorySource::Dot, None)
            .await
            .unwrap();
        let fact = memories
            .project_save(
                &root,
                NewFact::plain("Use pnpm to install packages.", MemorySource::Person, None),
            )
            .await
            .unwrap();
        store.close().await;
        (dot, own, fact)
    };
    // The store as it was before 0060: no index, and the migration not yet applied.
    let raw = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(&path))
        .await
        .unwrap();
    for sql in [
        "DROP TABLE memory_fts",
        "DROP TABLE memory_docs",
        "DELETE FROM _sqlx_migrations WHERE version = 60",
    ] {
        sqlx::query(sql).execute(&raw).await.unwrap();
    }
    raw.close().await;

    let store = Store::open(&path).await.unwrap();
    let memories = Memories::new(store.clone(), new_bus());
    assert_eq!(
        store.list_memories(&dot.id).await.unwrap(),
        vec![own.clone()]
    );
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let found = memories
        .call(&run.id, Tool::Recall, &json!({"query": "pnpm"}))
        .await
        .unwrap();
    assert_eq!(
        ids_of(&found),
        [own.id.as_str(), fact.id.as_str()],
        "{found}"
    );
    store.close().await;
}
