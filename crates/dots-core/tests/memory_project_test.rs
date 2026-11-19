//! A project's memory (memory spec sections 3, 4, 6 and 10): files in the project's folder
//! that are the truth, shared by the dots that work there and by no other, read as they are
//! after a person changed them by hand, and never written outside their folder.

mod common;

use std::path::{Path, PathBuf};

use common::{spec, temp_store};
use dots_core::events::{new_bus, RuntimeEvent};
use dots_core::memory::files::{self, FILES_RULE, INDEX_FILE};
use dots_core::memory::{
    Memories, NewFact, Tool, CORE_RULE, KIND_RULE, MAX_PER_PROJECT, NO_PROJECT, PERSONS,
    PROJECT_DUPLICATE, PROJECT_FULL, SCOPE_RULE, SECRET,
};
use dots_core::model::{
    Dot, Memory, MemoryKind, MemoryScope, MemorySource, NewRun, TriggerKind, WorkspaceMode,
};
use dots_core::store::Store;
use dots_core::Error;
use serde_json::json;
use tempfile::TempDir;

struct Setup {
    dir: TempDir,
    store: Store,
    memories: Memories,
    events: tokio::sync::broadcast::Receiver<RuntimeEvent>,
}

async fn setup() -> Setup {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let events = bus.subscribe();
    let memories = Memories::new(store.clone(), bus);
    Setup {
        dir,
        store,
        memories,
        events,
    }
}

impl Setup {
    /// A new folder under the test's own.
    fn folder(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// A dot that works in `folder`.
    async fn dot(&self, name: &str, folder: &Path) -> Dot {
        let mut s = spec(name);
        s.workdir = folder.to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        self.store.create_dot(&s, None, "default").await.unwrap()
    }

    /// A run of the dot, for a tool call.
    async fn run(&self, dot: &Dot) -> String {
        self.store
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap()
            .id
    }

    /// The projects whose memory changed since the last call, in order.
    fn changed(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            if let RuntimeEvent::ProjectMemoriesUpdated { root } = event {
                out.push(root);
            }
        }
        out
    }
}

fn fact(text: &str) -> NewFact<'_> {
    NewFact::plain(text, MemorySource::Person, None)
}

fn message(e: Error) -> String {
    match e {
        Error::Invalid(m) | Error::Conflict(m) | Error::NotFound(m) => m,
        other => other.to_string(),
    }
}

/// The names of the memory files of the project at `root`, sorted, without `MEMORY.md`.
fn file_names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(files::dir_of(root))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n != INDEX_FILE)
        .collect();
    names.sort();
    names
}

fn texts(list: &[Memory]) -> Vec<&str> {
    list.iter().map(|m| m.text.as_str()).collect()
}

/// Every file under `dir`, relative to it, with `/`.
fn tree(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let rel = path.strip_prefix(base).unwrap();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[tokio::test]
async fn a_project_memory_is_a_file_in_the_projects_folder_and_the_list_names_it() {
    let mut t = setup().await;
    let root = t.folder("shop");
    let dot = t.dot("builder", &root).await;
    // Nothing is made in the project until something is remembered.
    let empty = t.memories.project_list(&dot.id).await.unwrap().unwrap();
    assert!(empty.memories.is_empty());
    assert!(!files::dir_of(&root).exists());

    let files_named = vec!["ui\\package.json".to_string(), "./ui/src".to_string()];
    let saved = t
        .memories
        .project_save(
            &root,
            NewFact {
                text: "  Run the UI tests with bun x vitest run.  ",
                kind: MemoryKind::Procedural,
                files: &files_named,
                source: MemorySource::Dot,
                run_id: Some("01JRUN"),
            },
        )
        .await
        .unwrap();
    assert_eq!(saved.text, "Run the UI tests with bun x vitest run.");
    assert_eq!(saved.scope, MemoryScope::Project);
    assert_eq!(saved.files, ["ui/package.json", "ui/src"]);
    assert_eq!(saved.dot_id, "");

    let names = file_names(&root);
    assert_eq!(names.len(), 1);
    let tail = saved.id[saved.id.len() - 8..].to_ascii_lowercase();
    assert_eq!(names[0], format!("run-the-ui-tests-with-bun-{tail}.md"));
    let on_disk = std::fs::read_to_string(files::dir_of(&root).join(&names[0])).unwrap();
    assert_eq!(
        on_disk,
        format!(
            "---\nid: {}\nkind: procedural\ncreated: {}\nupdated: {}\nsource: sol\nrun: 01JRUN\n\
             pinned: false\nfiles:\n  - ui/package.json\n  - ui/src\n---\n\
             Run the UI tests with bun x vitest run.\n",
            saved.id, saved.created_at, saved.updated_at
        )
    );
    // Times are ISO 8601 in UTC.
    assert!(chrono::DateTime::parse_from_rfc3339(&saved.created_at).is_ok());
    assert!(saved.created_at.ends_with('Z'));
    let index = std::fs::read_to_string(files::dir_of(&root).join(INDEX_FILE)).unwrap();
    assert!(index.starts_with("# Project memory\n"), "{index}");
    assert!(
        index.contains(&format!(
            "\n## How-to\n\n- [{}] Run the UI tests with bun x vitest run. ({})\n",
            saved.id, names[0]
        )),
        "{index}"
    );
    // No temporary file is left beside them.
    assert_eq!(tree(&root).len(), 2, "{:?}", tree(&root));

    let listed = t.memories.project_list(&dot.id).await.unwrap().unwrap();
    assert_eq!(listed.memories, vec![saved.clone()]);
    assert_eq!(listed.skipped, 0);
    // The list names the project's folder as it resolves (on macOS a temporary folder is
    // under `/var`, a link to `/private/var`), so the two are compared resolved.
    let resolved = |path: &Path| std::fs::canonicalize(path).unwrap();
    assert_eq!(
        resolved(Path::new(&listed.dir)),
        resolved(&files::dir_of(&root))
    );
    assert_eq!(resolved(Path::new(&listed.root)), resolved(&root));
    assert_eq!(t.changed().len(), 1);

    // Changed: the same file, the new text, a later time; pinned and made core.
    tokio::time::sleep(std::time::Duration::from_millis(3)).await;
    let changed = t
        .memories
        .project_update(
            &root,
            &saved.id,
            Some("Run the UI tests with bun x vitest."),
            Some(true),
            Some(MemoryKind::Core),
        )
        .await
        .unwrap();
    assert!(changed.pinned && changed.kind == MemoryKind::Core);
    assert_eq!(changed.created_at, saved.created_at);
    assert!(changed.updated_at > saved.updated_at);
    assert_eq!(changed.files, saved.files);
    assert_eq!(file_names(&root), names);
    let index = std::fs::read_to_string(files::dir_of(&root).join(INDEX_FILE)).unwrap();
    assert!(index.contains("\n## Core\n\n"), "{index}");

    // Deleted: the file goes, the list is rewritten.
    let gone = t.memories.project_delete(&root, &saved.id).await.unwrap();
    assert_eq!(gone.id, saved.id);
    assert!(file_names(&root).is_empty());
    let again = t.memories.project_delete(&root, &saved.id).await;
    assert!(matches!(again, Err(Error::NotFound(_))), "{again:?}");
    assert_eq!(t.changed().len(), 2);
}

#[tokio::test]
async fn the_rules_of_a_memory_hold_for_a_projects_too() {
    let t = setup().await;
    let root = t.folder("shop");
    let save = |text: &'static str, kind: MemoryKind| {
        let (memories, root) = (t.memories.clone(), root.clone());
        async move {
            let new = NewFact { kind, ..fact(text) };
            memories.project_save(&root, new).await.map_err(message)
        }
    };
    save("We deploy on Fridays.", MemoryKind::Semantic)
        .await
        .unwrap();
    assert_eq!(
        save(" we  DEPLOY on fridays. ", MemoryKind::Semantic)
            .await
            .unwrap_err(),
        PROJECT_DUPLICATE
    );
    assert_eq!(
        save("password: hunter2hunter2", MemoryKind::Semantic)
            .await
            .unwrap_err(),
        SECRET
    );
    assert_eq!(
        save("", MemoryKind::Semantic).await.unwrap_err(),
        "A memory is 1 to 500 characters."
    );
    let long: &'static str = Box::leak("x".repeat(161).into_boxed_str());
    assert_eq!(save(long, MemoryKind::Core).await.unwrap_err(), CORE_RULE);
    save(long, MemoryKind::Semantic).await.unwrap();
    // Nothing refused left a file.
    assert_eq!(file_names(&root).len(), 2);

    // A change is held to the same rules.
    let list = t.memories.project_memories(&root).await.unwrap();
    let friday = list.iter().find(|m| m.text.starts_with("We")).unwrap();
    let long_one = list.iter().find(|m| m.text.starts_with('x')).unwrap();
    let update = |id: String, text: Option<&'static str>, kind: Option<MemoryKind>| {
        let (memories, root) = (t.memories.clone(), root.clone());
        async move {
            memories
                .project_update(&root, &id, text, None, kind)
                .await
                .map_err(message)
        }
    };
    assert_eq!(
        update(friday.id.clone(), Some(long), None)
            .await
            .unwrap_err(),
        PROJECT_DUPLICATE
    );
    assert_eq!(
        update(friday.id.clone(), Some("token = abcdefgh12345678"), None)
            .await
            .unwrap_err(),
        SECRET
    );
    assert_eq!(
        update(long_one.id.clone(), None, Some(MemoryKind::Core))
            .await
            .unwrap_err(),
        CORE_RULE
    );
    assert_eq!(t.memories.project_memories(&root).await.unwrap(), list);
}

#[tokio::test]
async fn a_project_keeps_at_most_300_memories_sage_writes() {
    assert_eq!(MAX_PER_PROJECT, 300);
    let t = setup().await;
    let root = t.folder("shop");
    let first = t
        .memories
        .project_save(&root, fact("Fact number 0."))
        .await
        .unwrap();
    // The files are the truth: 299 more written beside it count as the project's.
    for i in 1..MAX_PER_PROJECT {
        let memory = Memory {
            id: format!("01JTESTFACT{i:04}"),
            text: format!("Fact number {i}."),
            ..first.clone()
        };
        files::save_new(&root, &memory).unwrap();
    }
    let full = t
        .memories
        .project_save(&root, fact("One too many."))
        .await
        .unwrap_err();
    assert_eq!(message(full), PROJECT_FULL);
    assert_eq!(file_names(&root).len(), MAX_PER_PROJECT);
    t.memories.project_delete(&root, &first.id).await.unwrap();
    t.memories
        .project_save(&root, fact("Room again."))
        .await
        .unwrap();
}

#[tokio::test]
async fn files_changed_by_hand_are_read_as_they_are() {
    let t = setup().await;
    let root = t.folder("shop");
    let dot = t.dot("builder", &root).await;
    let kept = t
        .memories
        .project_save(&root, fact("The API runs on port 4000."))
        .await
        .unwrap();
    let doomed = t
        .memories
        .project_save(&root, fact("The old API ran on port 3000."))
        .await
        .unwrap();
    let dir = files::dir_of(&root);
    let name_of = |id: &str| {
        let loaded = files::load(&root);
        loaded.find(id).unwrap().file_name.clone()
    };

    // Edited by hand: the new text is the memory, under the same id.
    let kept_file = dir.join(name_of(&kept.id));
    let edited = std::fs::read_to_string(&kept_file)
        .unwrap()
        .replace("port 4000", "port 4100");
    std::fs::write(&kept_file, edited).unwrap();
    // Deleted by hand.
    std::fs::remove_file(dir.join(name_of(&doomed.id))).unwrap();
    // Written by hand, with no frontmatter at all.
    std::fs::write(dir.join("my-note.md"), "Ask before changing the schema.\n").unwrap();
    // Files that are no memory: nothing in them, a frontmatter that never closes, bytes that
    // are no text, a secret, a file far too large, and a folder.
    std::fs::write(dir.join("empty.md"), "").unwrap();
    std::fs::write(
        dir.join("open.md"),
        "---\nid: abcd\nkind: core\nNever closed",
    )
    .unwrap();
    std::fs::write(dir.join("binary.md"), [0xff, 0xfe, 0x00, 0xc3, 0x28]).unwrap();
    std::fs::write(dir.join("key.md"), "The key is sk-abcdefghijklmnop1234").unwrap();
    std::fs::write(dir.join("huge.md"), "x".repeat(20 * 1024)).unwrap();
    std::fs::create_dir(dir.join("folder.md")).unwrap();
    // And files that are not looked at: another type, a hidden one.
    std::fs::write(dir.join("notes.txt"), "Not a memory.").unwrap();
    std::fs::write(dir.join(".hidden.md"), "Not a memory.").unwrap();

    let listed = t.memories.project_list(&dot.id).await.unwrap().unwrap();
    let mut found = texts(&listed.memories);
    found.sort();
    assert_eq!(
        found,
        [
            "Ask before changing the schema.",
            "The API runs on port 4100."
        ]
    );
    assert_eq!(listed.skipped, 6);
    let edited = listed.memories.iter().find(|m| m.id == kept.id).unwrap();
    assert_eq!(edited.source, MemorySource::Person);
    let by_hand = listed.memories.iter().find(|m| m.id != kept.id).unwrap();
    assert_eq!(
        (by_hand.kind, by_hand.source, by_hand.pinned),
        (MemoryKind::Semantic, MemorySource::Person, false)
    );
    // The list catches up with the files when the tab reads them.
    let index = std::fs::read_to_string(dir.join(INDEX_FILE)).unwrap();
    assert!(
        index.contains("port 4100") && index.contains("(my-note.md)"),
        "{index}"
    );
    assert!(
        !index.contains("port 3000") && !index.contains("sk-"),
        "{index}"
    );

    // A run reads the same, and a secret a person left in a file never reaches it.
    let readable = t.memories.readable(&dot).await.unwrap();
    assert_eq!(readable.len(), 2);
    let run = t.run(&dot).await;
    let recalled = t
        .memories
        .call(&run, Tool::Recall, &json!({}))
        .await
        .unwrap();
    assert!(
        recalled.contains("port 4100") && !recalled.contains("sk-"),
        "{recalled}"
    );

    // The hand-written file can be changed through Sage: it gets its frontmatter, keeps its
    // name and its id.
    let changed = t
        .memories
        .project_update(&root, &by_hand.id, None, Some(true), None)
        .await
        .unwrap();
    assert_eq!(changed.id, by_hand.id);
    let text = std::fs::read_to_string(dir.join("my-note.md")).unwrap();
    assert!(
        text.starts_with(&format!("---\nid: {}\n", by_hand.id)),
        "{text}"
    );
    assert!(
        text.ends_with("---\nAsk before changing the schema.\n"),
        "{text}"
    );

    // The whole folder deleted by hand: no memory, no error, and the next one makes it again.
    std::fs::remove_dir_all(root.join(".sage")).unwrap();
    assert!(t.memories.readable(&dot).await.unwrap().is_empty());
    let listed = t.memories.project_list(&dot.id).await.unwrap().unwrap();
    assert_eq!((listed.memories.len(), listed.skipped), (0, 0));
    t.memories
        .project_save(&root, fact("Back again."))
        .await
        .unwrap();
    assert_eq!(file_names(&root).len(), 1);
}

#[tokio::test]
async fn nothing_a_model_passes_reaches_outside_the_memory_folder() {
    let t = setup().await;
    let root = t.folder("shop");
    let outside = t.folder("outside");
    std::fs::write(outside.join("victim.md"), "Do not touch.").unwrap();
    std::fs::write(root.join("victim.md"), "Do not touch.").unwrap();
    let dot = t.dot("builder", &root).await;
    let run = t.run(&dot).await;
    let call = |tool: Tool, input: serde_json::Value| {
        let (memories, run) = (t.memories.clone(), run.clone());
        async move { memories.call(&run, tool, &input).await }
    };

    // A text made of path parts gives a plain file name inside the folder.
    for text in [
        "../../outside/victim.md",
        "..\\..\\outside\\victim",
        "C:\\Windows\\System32\\drivers\\etc\\hosts",
        "CON",
        "/etc/passwd is where users are",
        "日本語のメモ",
        "....",
    ] {
        let answer = call(Tool::Remember, json!({"text": text, "scope": "project"}))
            .await
            .unwrap();
        assert!(answer.starts_with("Remembered "), "{answer}");
    }
    let names = file_names(&root);
    assert_eq!(names.len(), 7, "{names:?}");
    for name in &names {
        assert!(
            name.ends_with(".md")
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.'),
            "{name}"
        );
    }

    // A file outside the project is refused, and nothing is saved.
    for bad in [
        json!(["../outside/victim.md"]),
        json!(["/etc/passwd"]),
        json!(["C:/Windows/win.ini"]),
        json!(["src/../../x"]),
        json!(["\\\\server\\share\\x"]),
        json!(["ok.rs", "a\nb"]),
        json!("src/main.rs"),
        json!([1, 2]),
        json!(["a", "b", "c", "d", "e", "f", "g", "h", "i"]),
    ] {
        let refused = call(
            Tool::Remember,
            json!({"text": "A fact about files.", "files": bad}),
        )
        .await
        .unwrap_err();
        assert_eq!(refused, FILES_RULE);
    }
    assert_eq!(file_names(&root).len(), 7);
    assert!(t.store.list_memories(&dot.id).await.unwrap().is_empty());

    // An id that is a path names no memory: nothing is read, changed or deleted by it.
    for id in [
        "../../../outside/victim",
        "..\\..\\..\\outside\\victim.md",
        "victim.md",
        "../victim",
        "MEMORY.md",
        "",
    ] {
        assert_eq!(
            call(Tool::Forget, json!({"id": id})).await.unwrap_err(),
            format!("No memory {}.", id.trim())
        );
        let recalled = call(Tool::Recall, json!({"ids": [id]})).await.unwrap();
        assert_eq!(recalled, format!("No memory {}.", id.trim()));
        let update = t
            .memories
            .project_update(&root, id, Some("Changed."), None, None)
            .await;
        assert!(matches!(update, Err(Error::NotFound(_))), "{update:?}");
        let delete = t.memories.project_delete(&root, id).await;
        assert!(matches!(delete, Err(Error::NotFound(_))), "{delete:?}");
    }
    // A file name that is a path is refused by the files themselves.
    let any = t.memories.project_memories(&root).await.unwrap()[0].clone();
    for name in [
        "../victim.md",
        "..\\victim.md",
        "sub/x.md",
        "C:x.md",
        "x.txt",
        "MEMORY.md",
    ] {
        assert!(files::rewrite(&root, name, &any).is_err(), "{name}");
        assert!(files::remove(&root, name).is_err(), "{name}");
    }
    assert_eq!(
        std::fs::read_to_string(outside.join("victim.md")).unwrap(),
        "Do not touch."
    );
    assert_eq!(
        std::fs::read_to_string(root.join("victim.md")).unwrap(),
        "Do not touch."
    );
    assert_eq!(tree(&outside), ["victim.md"]);
    // The project has its memory folder and the file that was there, nothing else.
    assert!(tree(&root)
        .iter()
        .all(|f| f == "victim.md" || f.starts_with(".sage/memory/")));
    assert_eq!(file_names(&root).len(), 7);
}

#[tokio::test]
async fn a_memory_folder_that_leads_out_of_the_project_is_not_used() {
    let t = setup().await;
    let root = t.folder("shop");
    let outside = t.folder("outside");
    std::fs::write(outside.join("theirs.md"), "Another place's note.").unwrap();
    // `.sage` is a link to a folder outside the project.
    let link = root.join(".sage");
    let target = t.folder("elsewhere");
    std::fs::create_dir_all(target.join("memory")).unwrap();
    std::fs::write(target.join("memory").join("theirs.md"), "Elsewhere's note.").unwrap();
    #[cfg(windows)]
    let linked = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .output()
        .is_ok_and(|o| o.status.success());
    #[cfg(not(windows))]
    let linked = std::os::unix::fs::symlink(&target, &link).is_ok();
    if !linked {
        eprintln!("skipped: this system does not let the test make a link");
        return;
    }
    let dot = t.dot("builder", &root).await;
    assert!(t.memories.readable(&dot).await.unwrap().is_empty());
    let refused = t.memories.project_save(&root, fact("A fact.")).await;
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.to_string().contains(files::OUTSIDE)),
        "{refused:?}"
    );
    assert_eq!(tree(&target), ["memory/theirs.md"]);
}

#[tokio::test]
async fn dots_of_one_project_share_its_memory_and_a_dot_elsewhere_does_not() {
    let t = setup().await;
    // A git checkout: a dot in a subfolder works in the same project as one at the top.
    let repo = t.folder("repo");
    std::fs::create_dir_all(repo.join("ui")).unwrap();
    common::git_out(&repo, &["init", "-q"]);
    let other_root = t.folder("other");
    let builder = t.dot("builder", &repo).await;
    let reviewer = t.dot("reviewer", &repo.join("ui")).await;
    let stranger = t.dot("stranger", &other_root).await;
    let homeless = t.dot("homeless", &t.dir.path().join("gone")).await;
    let (b_run, r_run, s_run, h_run) = (
        t.run(&builder).await,
        t.run(&reviewer).await,
        t.run(&stranger).await,
        t.run(&homeless).await,
    );
    let m = &t.memories;

    // The builder saves one fact for the project (it names a file) and one for itself.
    let project_fact = m
        .call(
            &b_run,
            Tool::Remember,
            &json!({"text": "The UI is built with Vite.", "files": ["ui/vite.config.ts"]}),
        )
        .await
        .unwrap();
    let id = project_fact
        .strip_prefix("Remembered ")
        .and_then(|s| s.strip_suffix(" for the project."))
        .unwrap_or_else(|| panic!("{project_fact}"))
        .to_string();
    let own = m
        .call(
            &b_run,
            Tool::Remember,
            &json!({"text": "I am the builder."}),
        )
        .await
        .unwrap();
    assert!(own.starts_with("Remembered ") && own.ends_with('.') && !own.contains("project"));
    // The memory is in the main checkout's folder, whichever subfolder a dot works in.
    assert_eq!(file_names(&repo).len(), 1);
    assert!(!repo.join("ui").join(".sage").exists());

    // The reviewer reads the project's fact, not the builder's own; the stranger reads neither.
    let recall = |run: &str| {
        let (m, run) = (m.clone(), run.to_string());
        async move { m.call(&run, Tool::Recall, &json!({})).await.unwrap() }
    };
    assert_eq!(
        recall(&r_run).await,
        format!("[{id}] The UI is built with Vite. (files: ui/vite.config.ts)")
    );
    assert!(recall(&b_run).await.contains("I am the builder."));
    assert_eq!(recall(&s_run).await, "Nothing remembered about that.");
    assert_eq!(recall(&h_run).await, "Nothing remembered about that.");
    let by_id = |run: &str| {
        let (m, run, id) = (m.clone(), run.to_string(), id.clone());
        async move {
            m.call(&run, Tool::Recall, &json!({"ids": [id]}))
                .await
                .unwrap()
        }
    };
    assert_eq!(
        by_id(&r_run).await,
        format!("[{id}] The UI is built with Vite. (files: ui/vite.config.ts)")
    );
    assert_eq!(by_id(&s_run).await, format!("No memory {id}."));
    assert_eq!(
        texts(&m.readable(&reviewer).await.unwrap()),
        ["The UI is built with Vite."]
    );
    assert!(m.readable(&stranger).await.unwrap().is_empty());
    assert_eq!(
        m.project_list(&reviewer.id)
            .await
            .unwrap()
            .unwrap()
            .memories
            .len(),
        1
    );
    assert!(m.project_list(&homeless.id).await.unwrap().is_none());

    // The stranger cannot forget it; a dot of the project can, though another dot saved it.
    assert_eq!(
        m.call(&s_run, Tool::Forget, &json!({"id": id}))
            .await
            .unwrap_err(),
        format!("No memory {id}.")
    );
    assert_eq!(file_names(&repo).len(), 1);
    // What the person wrote in the project, no dot forgets.
    let persons = m
        .project_save(&repo, fact("Never push to main."))
        .await
        .unwrap();
    assert_eq!(
        m.call(&r_run, Tool::Forget, &json!({"id": persons.id}))
            .await
            .unwrap_err(),
        PERSONS
    );
    assert_eq!(
        m.call(&r_run, Tool::Forget, &json!({"id": id}))
            .await
            .unwrap(),
        format!("Forgot {id}.")
    );
    assert_eq!(
        texts(&m.project_memories(&repo).await.unwrap()),
        ["Never push to main."]
    );

    // Where a memory goes: the project when asked, or when it names files; else the dot. A
    // dot without a folder has no project to save to.
    let said = m
        .call(
            &r_run,
            Tool::Remember,
            &json!({"text": "Reviews need two approvals.", "scope": "project", "kind": "core"}),
        )
        .await
        .unwrap();
    assert!(said.ends_with(" for the project."), "{said}");
    let kept = m
        .call(
            &r_run,
            Tool::Remember,
            &json!({"text": "I review ui/ only.", "scope": "sol", "files": ["ui"]}),
        )
        .await
        .unwrap();
    assert!(!kept.contains("project"), "{kept}");
    let own = t.store.list_memories(&reviewer.id).await.unwrap();
    assert_eq!(
        (own.len(), own[0].files.clone()),
        (1, vec!["ui".to_string()])
    );
    assert_eq!(own[0].scope, MemoryScope::Dot);
    let project = m.project_memories(&repo).await.unwrap();
    assert!(project
        .iter()
        .any(|p| p.kind == MemoryKind::Core && p.source == MemorySource::Dot));
    assert_eq!(
        m.call(
            &h_run,
            Tool::Remember,
            &json!({"text": "A fact.", "scope": "project"})
        )
        .await
        .unwrap_err(),
        NO_PROJECT
    );
    // Without a folder, a fact that names files is the dot's own.
    let homeless_fact = m
        .call(
            &h_run,
            Tool::Remember,
            &json!({"text": "A fact about a file.", "files": ["a.rs"]}),
        )
        .await
        .unwrap();
    assert!(!homeless_fact.contains("project"), "{homeless_fact}");
    for (input, rule) in [
        (json!({"text": "x", "kind": "important"}), KIND_RULE),
        (json!({"text": "x", "scope": "world"}), SCOPE_RULE),
        (
            json!({"text": "password=hunter2hunter2", "scope": "project"}),
            SECRET,
        ),
        (json!({"text": "x".repeat(161), "kind": "core"}), CORE_RULE),
    ] {
        assert_eq!(
            m.call(&r_run, Tool::Remember, &input).await.unwrap_err(),
            rule
        );
    }
}

/// Memory spec section 3: a dot whose folder is a linked worktree of a repository works in
/// that repository's project, whose folder is not above its own; and a dot in a subfolder of
/// a project reads the memory kept at the project's top.
#[tokio::test]
async fn a_linked_worktree_and_a_subfolder_read_the_main_checkouts_memory() {
    let t = setup().await;
    let repo = t.folder("repo");
    std::fs::create_dir_all(repo.join("crates").join("core")).unwrap();
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "t"],
        vec!["add", "a.txt"],
        vec!["commit", "-q", "-m", "first"],
    ] {
        common::git_out(&repo, &args);
    }
    let linked = t.dir.path().join("linked");
    common::git_out(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            linked.to_str().unwrap(),
            "-b",
            "side",
        ],
    );
    let fact = t
        .memories
        .project_save(&repo, fact("The core crate has no network code."))
        .await
        .unwrap();
    let in_worktree = t.dot("side", &linked).await;
    let in_subfolder = t.dot("core", &repo.join("crates").join("core")).await;
    for dot in [&in_worktree, &in_subfolder] {
        let readable = t.memories.readable(dot).await.unwrap();
        assert_eq!(readable, vec![fact.clone()], "{}", dot.spec.name);
    }
    // What the worktree's dot saves for the project is in the main checkout, not in its own
    // folder.
    let run = t.run(&in_worktree).await;
    t.memories
        .call(
            &run,
            Tool::Remember,
            &json!({"text": "Branches are cut from main.", "scope": "project"}),
        )
        .await
        .unwrap();
    assert_eq!(file_names(&repo).len(), 2);
    assert!(!linked.join(".sage").exists());
    // A folder with no memory at or above it is not a project anyone has to look for.
    let plain = t.dot("plain", &t.folder("plain")).await;
    assert!(t.memories.readable(&plain).await.unwrap().is_empty());
}
