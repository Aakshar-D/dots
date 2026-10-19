use std::path::Path;

use dots_core::context::{self, Collection, MAX_DOCUMENTS, MAX_OUTPUT};
use serde_json::json;

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Creates a directory link at `link` pointing to `target` (a junction on Windows).
fn dir_link(target: &Path, link: &Path) {
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

/// A collection named `company` of the folder `root`.
fn company(root: &Path) -> Vec<Collection> {
    vec![Collection {
        name: "company".into(),
        dir: root.to_path_buf(),
        description: "How we work".into(),
        documents: 0,
    }]
}

#[test]
fn the_walk_finds_documents_by_extension_in_sorted_order() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for rel in [
        "b.md",
        "A.TXT",
        "notes/c.markdown",
        "notes/deep/d.yaml",
        "data.csv",
        "conf.toml",
        "x.json",
        "y.yml",
        "z.rst",
        "w.adoc",
        "v.org",
        "app.exe",
        "logo.png",
        "README",
        ".hidden.md",
        ".git/config.md",
    ] {
        write(root, rel, "text");
    }
    let walk = context::walk(root).unwrap();
    assert_eq!(
        walk.paths,
        [
            "A.TXT",
            "b.md",
            "conf.toml",
            "data.csv",
            "notes/c.markdown",
            "notes/deep/d.yaml",
            "v.org",
            "w.adoc",
            "x.json",
            "y.yml",
            "z.rst",
        ]
    );
    assert_eq!(walk.total, 11);
}

#[test]
fn the_walk_leaves_out_folder_links() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "secret.md", "secret");
    write(dir.path(), "a.md", "a");
    dir_link(outside.path(), &dir.path().join("linked"));
    assert_eq!(context::walk(dir.path()).unwrap().paths, ["a.md"]);
}

#[test]
fn the_walk_stops_listing_at_the_bound_and_counts_all() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..=MAX_DOCUMENTS {
        write(dir.path(), &format!("d{i:05}.md"), "x");
    }
    let walk = context::walk(dir.path()).unwrap();
    assert_eq!(walk.paths.len(), MAX_DOCUMENTS);
    assert_eq!(walk.total, MAX_DOCUMENTS + 1);
    assert_eq!(walk.paths[0], "d00000.md");
    let list = context::list(dir.path()).unwrap();
    assert!(
        list.contains("the collection has 2001 documents; only the first 2000"),
        "{}",
        &list[list.len().saturating_sub(300)..]
    );
}

#[test]
fn a_missing_folder_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = context::walk(&dir.path().join("gone")).unwrap_err();
    assert!(err.starts_with("the folder cannot be read"), "{err}");
}

#[test]
fn titles_and_summaries_come_from_frontmatter_then_text_then_the_name() {
    let fm = context::describe(
        "people/ana.md",
        "---\ntitle: Ana Lima\ndescription: Owns billing\n---\n# Ignored\nBody",
    );
    assert_eq!(
        (fm.title.as_str(), fm.summary.as_str()),
        ("Ana Lima", "Owns billing")
    );
    let heading = context::describe("style.md", "\n# House style\n\n## Tone\nShort sentences.\n");
    assert_eq!(
        (heading.title.as_str(), heading.summary.as_str()),
        ("House style", "Short sentences.")
    );
    let plain = context::describe("notes/on-call.txt", "\n  First line here.  \nSecond");
    assert_eq!(
        (plain.title.as_str(), plain.summary.as_str()),
        ("on-call", "First line here.")
    );
    assert_eq!(plain.path, "notes/on-call.txt");
    // A frontmatter that does not parse gives no fields.
    let broken = context::describe("b.md", "---\ntitle: [unclosed\n---\n# Real\ntext");
    assert_eq!(broken.title, "Real");
    let long = context::describe("l.md", &"é".repeat(300));
    assert_eq!(long.summary.chars().count(), 201);
    assert!(long.summary.ends_with('…'));
}

#[test]
fn the_list_shows_one_line_per_document() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "style.md", "# House style\nShort sentences.");
    write(dir.path(), "empty.txt", "");
    assert_eq!(
        context::list(dir.path()).unwrap(),
        "- empty.txt — empty\n- style.md — House style: Short sentences."
    );
    let none = tempfile::tempdir().unwrap();
    assert_eq!(
        context::list(none.path()).unwrap(),
        "The collection has no documents."
    );
}

#[test]
fn a_long_list_is_capped_and_says_what_it_left_out() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..400 {
        write(
            dir.path(),
            &format!("doc{i:03}.md"),
            &format!("# Title {i}\n{}", "word ".repeat(40)),
        );
    }
    let list = context::list(dir.path()).unwrap();
    assert!(list.len() <= MAX_OUTPUT, "{}", list.len());
    assert!(
        list.contains("more documents are not listed; use query"),
        "{list}"
    );
}

#[test]
fn a_search_needs_every_word_and_ranks_by_hits() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "billing.md",
        "# Billing\nInvoices go out on the 1st.\nAsk Ana about invoices.\nInvoices, invoices!",
    );
    write(
        dir.path(),
        "people/ana.md",
        "# Ana\nAna owns invoices and billing.",
    );
    write(dir.path(), "style.md", "# Style\nNothing about money.");
    let out = context::search(dir.path(), "INVOICES ana").unwrap();
    assert!(
        out.starts_with("2 documents match \"INVOICES ana\":"),
        "{out}"
    );
    let billing = out.find("billing.md — Billing").unwrap();
    let ana = out.find("people/ana.md — Ana").unwrap();
    // billing.md: 4 hits of "invoices" and 1 of "ana". people/ana.md: 1 and 4 (its text, path
    // and title), so 5 against 6.
    assert!(billing < ana, "{out}");
    assert!(
        out.contains("  line 2: Invoices go out on the 1st."),
        "{out}"
    );
    assert!(out.contains("  line 3: Ask Ana about invoices."), "{out}");
    assert!(!out.contains("style.md"), "{out}");
}

#[test]
fn a_word_in_the_path_or_title_matches() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "runbooks/deploy.md", "Steps to ship.");
    let out = context::search(dir.path(), "runbooks").unwrap();
    assert!(out.starts_with("1 documents match"), "{out}");
    assert!(out.contains("runbooks/deploy.md — deploy"), "{out}");
}

#[test]
fn a_search_shows_the_ten_best_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..12 {
        write(dir.path(), &format!("d{i:02}.md"), &"alpha ".repeat(i + 1));
    }
    let out = context::search(dir.path(), "alpha").unwrap();
    assert!(
        out.starts_with("12 documents match \"alpha\"; the 10 best:"),
        "{out}"
    );
    assert!(out.find("d11.md").unwrap() < out.find("d10.md").unwrap());
    assert!(!out.contains("d01.md"), "{out}");
    assert!(!out.contains("d00.md"), "{out}");
}

#[test]
fn a_search_with_no_match_or_no_word_says_so() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.md", "alpha");
    assert_eq!(
        context::search(dir.path(), "beta").unwrap(),
        "No document matches \"beta\"."
    );
    assert!(context::search(dir.path(), " a b ").is_err());
}

#[test]
fn a_document_is_read_from_an_offset() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "notes/a.md", "one\ntwo\nthree\n");
    assert_eq!(
        context::read(dir.path(), "notes/a.md", 1).unwrap(),
        "one\ntwo\nthree"
    );
    assert_eq!(
        context::read(dir.path(), "notes/a.md", 2).unwrap(),
        "two\nthree"
    );
    assert_eq!(
        context::read(dir.path(), "./notes/a.md", 3).unwrap(),
        "three"
    );
    let past = context::read(dir.path(), "notes/a.md", 9).unwrap_err();
    assert!(
        past.contains("has 3 lines; offset 9 is past the end"),
        "{past}"
    );
}

#[test]
fn a_long_document_is_cut_with_the_offset_to_go_on() {
    let dir = tempfile::tempdir().unwrap();
    let text: String = (1..=2000)
        .map(|i| format!("line number {i:04}\n"))
        .collect();
    write(dir.path(), "long.md", &text);
    let first = context::read(dir.path(), "long.md", 1).unwrap();
    assert!(first.len() <= MAX_OUTPUT);
    let next: usize = first
        .rsplit("continue with offset=")
        .next()
        .unwrap()
        .trim_end_matches(']')
        .parse()
        .unwrap();
    let last_shown = first.lines().rev().nth(1).unwrap();
    assert_eq!(last_shown, format!("line number {:04}", next - 1));
    let rest = context::read(dir.path(), "long.md", next).unwrap();
    assert!(
        rest.starts_with(&format!("line number {next:04}")),
        "{rest}"
    );
    // One line longer than a result: its start, then the next line.
    write(
        dir.path(),
        "wide.md",
        &format!("{}\nafter", "x".repeat(40_000)),
    );
    let wide = context::read(dir.path(), "wide.md", 1).unwrap();
    assert!(wide.len() <= MAX_OUTPUT);
    assert!(
        wide.ends_with("[truncated: continue with offset=2]"),
        "{}",
        &wide[wide.len() - 60..]
    );
    assert_eq!(context::read(dir.path(), "wide.md", 2).unwrap(), "after");
}

#[test]
fn reading_outside_the_collection_or_a_non_document_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("kb");
    write(&root, "a.md", "a");
    write(&root, "tool.exe", "MZ");
    write(&root, ".secret.md", "hidden");
    write(&root, "sub/b.md", "b");
    write(dir.path(), "outside.md", "secret");
    let outside_dir = tempfile::tempdir().unwrap();
    write(outside_dir.path(), "x.md", "secret");
    dir_link(outside_dir.path(), &root.join("linked"));
    let absolute = dir.path().join("outside.md").to_string_lossy().into_owned();
    for rel in [
        "../outside.md",
        "sub/../../outside.md",
        absolute.as_str(),
        "C:outside.md",
        "a.md:stream",
        ".secret.md",
        "linked/x.md",
        "",
    ] {
        let err = context::read(&root, rel, 1).unwrap_err();
        assert!(err.contains("is not inside the collection"), "{rel}: {err}");
    }
    let exe = context::read(&root, "tool.exe", 1).unwrap_err();
    assert_eq!(exe, "tool.exe is not a document of the collection");
    let gone = context::read(&root, "gone.md", 1).unwrap_err();
    assert_eq!(gone, "gone.md was not found in the collection");
    let folder = context::read(&root, "sub", 1).unwrap_err();
    assert!(folder.contains("is a folder"), "{folder}");
}

#[test]
fn a_call_lists_searches_or_reads_by_its_arguments() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "style.md", "# Style\nShort sentences.");
    let c = company(dir.path());
    assert_eq!(
        context::call(&c, &json!({"collection": "company"})).unwrap(),
        "- style.md — Style: Short sentences."
    );
    assert!(
        context::call(&c, &json!({"collection": "company", "query": "short"}))
            .unwrap()
            .starts_with("1 documents match")
    );
    assert_eq!(
        context::call(
            &c,
            &json!({"collection": "company", "path": "style.md", "offset": 2})
        )
        .unwrap(),
        "Short sentences."
    );
    // Empty optional arguments are no arguments.
    assert_eq!(
        context::call(
            &c,
            &json!({"collection": " company ", "query": " ", "path": null})
        )
        .unwrap(),
        "- style.md — Style: Short sentences."
    );
}

#[test]
fn a_call_with_bad_arguments_says_what_is_wrong() {
    let dir = tempfile::tempdir().unwrap();
    let c = company(dir.path());
    let err = |input| context::call(&c, &input).unwrap_err();
    assert_eq!(err(json!({})), "collection is required");
    assert_eq!(
        err(json!({"collection": "clients"})),
        "unknown collection \"clients\". Collections: company"
    );
    assert_eq!(
        context::call(&[], &json!({"collection": "x"})).unwrap_err(),
        "unknown collection \"x\". This run has no collections."
    );
    assert!(
        err(json!({"collection": "company", "query": "a b", "path": "a.md"})).contains("not both")
    );
    for offset in [json!(0), json!(-1), json!(1.5), json!("2")] {
        assert_eq!(
            err(json!({"collection": "company", "path": "a.md", "offset": offset})),
            "offset must be a whole number of at least 1"
        );
    }
    let gone = company(&dir.path().join("gone"));
    assert_eq!(
        context::call(&gone, &json!({"collection": "company"})).unwrap_err(),
        "the folder of collection company is missing"
    );
}

#[test]
fn the_prompt_section_lists_the_collections() {
    assert_eq!(context::prompt_section(&[], "context"), "");
    let mut collections = vec![
        Collection {
            name: "company".into(),
            dir: "C:/kb".into(),
            description: "How we work".into(),
            documents: 42,
        },
        Collection {
            name: "clients".into(),
            dir: "C:/clients".into(),
            description: " ".into(),
            documents: 1,
        },
    ];
    let section = context::prompt_section(&collections, "mcp__dots__context");
    assert_eq!(
        section,
        "Shared context you can read. Call the tool `mcp__dots__context` with a collection \
         name to list its documents, `query` to search it, or `path` to read one. Look here \
         before you guess how the person works.\n\
         - company: How we work (42 documents)\n\
         - clients: (no description) (1 document)"
    );
    for i in 0..60 {
        collections.push(Collection {
            name: format!("c{i}"),
            dir: "C:/x".into(),
            description: String::new(),
            documents: 0,
        });
    }
    let section = context::prompt_section(&collections, "context");
    assert_eq!(section.lines().count(), 1 + context::MAX_PROMPT_COLLECTIONS);
}
