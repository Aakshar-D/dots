//! The skill creator: what a draft must hold, saving it in Dots' skills folder, and removing one
//! of Dots' own skills (skill creator spec §6, §7).

mod common;

use std::path::{Path, PathBuf};

use common::{spec, temp_store};
use dots_core::events::{new_bus, Bus};
use dots_core::library::{LibraryConfig, SkillConfig};
use dots_core::library_service::LibraryService;
use dots_core::skill_creator::{check_draft, DraftSummary, MAX_SKILL_BYTES};
use dots_core::store::Store;
use dots_core::{Config, Error, Runtime};

const DRAFT: &str = "---\nname: pr-tests\ndescription: \"Checks a PR for missing tests: use it \
                     on a review.\"\n---\n# Missing tests\n\nList the changed functions.\n";

fn service(store: &Store, bus: &Bus, skills: &Path) -> LibraryService {
    LibraryService::new(store.clone(), bus.clone()).with_skills_dir(skills.to_path_buf())
}

fn invalid(text: &str) -> String {
    match check_draft(text) {
        Err(Error::Invalid(m)) => m,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn a_draft_with_a_name_a_description_and_instructions_can_be_saved() {
    assert_eq!(
        check_draft(DRAFT).unwrap(),
        DraftSummary {
            name: "pr-tests".into(),
            description: "Checks a PR for missing tests: use it on a review.".into(),
        }
    );
    // A BOM and CRLF line ends, as an editor on Windows may leave them.
    let crlf = format!("\u{feff}{}", DRAFT.replace('\n', "\r\n"));
    assert_eq!(check_draft(&crlf).unwrap().name, "pr-tests");
}

#[test]
fn a_draft_says_what_to_change_when_it_cannot_be_saved() {
    assert!(invalid("name: x\n").contains("must start with a --- line"));
    assert!(invalid("---\nname: x\ndescription: y\nBody").contains("could not be read"));
    let twice = "---\nname: a\nname: b\ndescription: d\n---\nBody\n";
    assert!(invalid(twice).contains("could not be read"));
    let colon = "---\nname: a\ndescription: Checks: things\n---\nBody\n";
    assert!(invalid(colon).contains("could not be read"));
    let quote = "---\nname: a\ndescription: \"open\n---\nBody\n";
    assert!(invalid(quote).contains("could not be read"));
    assert!(invalid("---\ndescription: d\n---\nBody\n").contains("Add a name: line"));
    assert!(invalid("---\nname: a\n---\nBody\n").contains("Add a description: line"));
    assert!(invalid("---\nname: a\ndescription: d\n---\n\n  \n").contains("instructions"));
    assert!(invalid("---\nname: a b\ndescription: d\n---\nBody\n").contains("skill name"));
}

#[test]
fn a_name_windows_cannot_use_for_a_folder_is_refused() {
    for name in ["con", "NUL", "com0", "com1.md", "lpt9", "notes."] {
        let text = format!("---\nname: {name}\ndescription: d\n---\nBody\n");
        assert!(invalid(&text).contains("cannot be a folder name"), "{name}");
    }
    let text = "---\nname: console\ndescription: d\n---\nBody\n";
    assert_eq!(check_draft(text).unwrap().name, "console");
}

#[test]
fn a_draft_longer_than_the_limit_is_refused() {
    let long = format!("{DRAFT}{}", "x".repeat(MAX_SKILL_BYTES));
    assert!(invalid(&long).contains("longer than 64 KB"));
}

#[tokio::test]
async fn a_saved_draft_is_a_skill_in_dots_skills_folder() {
    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    let item = lib.create_skill(DRAFT).await.unwrap();
    assert_eq!(item.name, "pr-tests");
    let folder = skills.join("pr-tests");
    assert_eq!(
        item.config,
        LibraryConfig::Skill(SkillConfig {
            path: folder.to_string_lossy().into_owned()
        })
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("SKILL.md")).unwrap(),
        DRAFT
    );
    // Only the skill's folder is left: no hidden folder of the write.
    let names: Vec<String> = std::fs::read_dir(&skills)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["pr-tests"]);
    let list = lib.list().await.unwrap();
    assert!(list[0].dots_folder);
    assert_eq!(
        list[0].skill.as_ref().unwrap().description,
        "Checks a PR for missing tests: use it on a review."
    );
}

#[tokio::test]
async fn a_name_or_folder_that_exists_is_refused_and_nothing_is_overwritten() {
    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    lib.create_skill(DRAFT).await.unwrap();
    let again = DRAFT.replace("List the changed", "Name the changed");
    let err = lib.create_skill(&again).await.unwrap_err();
    assert!(
        matches!(&err, Error::Conflict(m) if m.contains("already in the Library")),
        "{err}"
    );
    // A folder of that name without a Library row: a person's own, or a save cut short.
    std::fs::create_dir_all(skills.join("csv-tools")).unwrap();
    std::fs::write(skills.join("csv-tools").join("notes.txt"), "mine").unwrap();
    let csv = DRAFT.replace("name: pr-tests", "name: csv-tools");
    let err = lib.create_skill(&csv).await.unwrap_err();
    assert!(
        matches!(&err, Error::Conflict(m) if m.contains("already exists in Dots' skills folder")),
        "{err}"
    );
    assert_eq!(
        std::fs::read_to_string(skills.join("pr-tests").join("SKILL.md")).unwrap(),
        DRAFT
    );
    assert!(!skills.join("csv-tools").join("SKILL.md").exists());
    assert_eq!(store.list_library_items().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_failed_add_leaves_no_folder_behind() {
    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("dots.db").display()
    ))
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER refuse BEFORE INSERT ON library_items \
         BEGIN SELECT RAISE(ABORT, 'refused'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    lib.create_skill(DRAFT).await.unwrap_err();
    assert!(!skills.join("pr-tests").exists());
}

#[tokio::test]
async fn without_a_skills_folder_nothing_is_saved() {
    let (_dir, store) = temp_store().await;
    let lib = LibraryService::new(store.clone(), new_bus());
    assert!(lib.create_skill(DRAFT).await.is_err());
    assert!(store.list_library_items().await.unwrap().is_empty());
}

#[tokio::test]
async fn removing_dots_own_skill_deletes_its_folder_and_no_other() {
    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    let own = lib.create_skill(DRAFT).await.unwrap();
    let theirs_dir = dir.path().join("mine").join("csv");
    std::fs::create_dir_all(&theirs_dir).unwrap();
    std::fs::write(
        theirs_dir.join("SKILL.md"),
        "---\nname: csv\ndescription: d\n---\nBody\n",
    )
    .unwrap();
    let theirs = lib
        .add(
            "csv",
            LibraryConfig::Skill(SkillConfig {
                path: theirs_dir.to_string_lossy().into_owned(),
            }),
            None,
        )
        .await
        .unwrap();
    let list = lib.list().await.unwrap();
    let flag = |name: &str| {
        list.iter()
            .find(|e| e.item.name == name)
            .unwrap()
            .dots_folder
    };
    assert!(flag("pr-tests") && !flag("csv"));

    lib.remove(&own.id, false).await.unwrap();
    assert!(!skills.join("pr-tests").exists());
    assert!(skills.exists());
    lib.remove(&theirs.id, false).await.unwrap();
    assert!(theirs_dir.join("SKILL.md").exists());
    // The name can be made again.
    lib.create_skill(DRAFT).await.unwrap();
}

#[tokio::test]
async fn a_skill_on_the_skills_folder_itself_is_not_dots_own() {
    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    lib.create_skill(DRAFT).await.unwrap();
    std::fs::write(
        skills.join("SKILL.md"),
        "---\nname: root\ndescription: d\n---\nBody\n",
    )
    .unwrap();
    let root = lib
        .add(
            "root",
            LibraryConfig::Skill(SkillConfig {
                path: skills.to_string_lossy().into_owned(),
            }),
            None,
        )
        .await
        .unwrap();
    let list = lib.list().await.unwrap();
    assert!(
        !list
            .iter()
            .find(|e| e.item.name == "root")
            .unwrap()
            .dots_folder
    );
    lib.remove(&root.id, false).await.unwrap();
    assert!(skills.join("SKILL.md").exists());
    assert!(skills.join("pr-tests").join("SKILL.md").exists());
}

#[tokio::test]
async fn a_folder_another_row_uses_is_kept_until_the_last_row_goes() {
    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    let first = lib.create_skill(DRAFT).await.unwrap();
    let second = lib
        .add(
            "pr-tests-copy",
            LibraryConfig::Skill(SkillConfig {
                path: skills.join("pr-tests").to_string_lossy().into_owned(),
            }),
            None,
        )
        .await
        .unwrap();
    lib.remove(&first.id, false).await.unwrap();
    assert!(skills.join("pr-tests").join("SKILL.md").exists());
    assert!(store.get_library_item(&second.id).await.is_ok());
    lib.remove(&second.id, false).await.unwrap();
    assert!(!skills.join("pr-tests").exists());
}

#[tokio::test]
async fn a_used_skill_is_not_deleted_until_the_person_confirms() {
    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    let own = lib.create_skill(DRAFT).await.unwrap();
    let mut s = spec("reviewer");
    s.uses.skills = vec!["pr-tests".into()];
    store.create_dot(&s, None).await.unwrap();
    assert!(matches!(
        lib.remove(&own.id, false).await,
        Err(Error::Conflict(_))
    ));
    assert!(skills.join("pr-tests").join("SKILL.md").exists());
    lib.remove(&own.id, true).await.unwrap();
    assert!(!skills.join("pr-tests").exists());
}

#[cfg(windows)]
#[tokio::test]
async fn a_folder_that_cannot_be_deleted_keeps_the_skill() {
    use std::os::windows::fs::OpenOptionsExt;

    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    let own = lib.create_skill(DRAFT).await.unwrap();
    // Open with no sharing, as an editor may: Windows refuses to delete it meanwhile.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(skills.join("pr-tests").join("SKILL.md"))
        .unwrap();
    let err = lib.remove(&own.id, false).await.unwrap_err();
    let text = err.to_string();
    assert!(text.contains("Could not delete"), "{text}");
    assert!(text.contains("pr-tests"), "{text}");
    assert!(text.contains("Close anything using it"), "{text}");
    assert!(store.get_library_item(&own.id).await.is_ok());
    drop(held);
    lib.remove(&own.id, false).await.unwrap();
    assert!(!skills.join("pr-tests").exists());
}

#[tokio::test]
async fn starting_dots_removes_saves_a_crash_cut_short() {
    let dir = tempfile::tempdir().unwrap();
    let data: PathBuf = dir.path().join("data");
    let skills = data.join("skills");
    let cut_short = skills.join(".pr-tests.01J0X0000000000000000000AB");
    std::fs::create_dir_all(&cut_short).unwrap();
    std::fs::write(cut_short.join("SKILL.md"), "half").unwrap();
    std::fs::create_dir_all(skills.join("csv")).unwrap();
    // A person's own hidden folders, and one that only looks like a save, are kept.
    std::fs::create_dir_all(skills.join(".git")).unwrap();
    std::fs::write(skills.join(".git").join("HEAD"), "ref").unwrap();
    std::fs::create_dir_all(skills.join(".notes")).unwrap();
    std::fs::create_dir_all(skills.join(".pr-tests.01J0X")).unwrap();
    let mut cfg = Config::new(&data);
    cfg.port = 0;
    // A missing program: no test may run a `claude` found on the PATH.
    cfg.claude_path = Some(dir.path().join("no-claude.exe"));
    let _rt = Runtime::start(cfg).await.unwrap();
    assert!(!cut_short.exists());
    assert!(skills.join("csv").exists());
    assert!(skills.join(".git").join("HEAD").exists());
    assert!(skills.join(".notes").exists());
    assert!(skills.join(".pr-tests.01J0X").exists());
}

#[cfg(windows)]
#[tokio::test]
async fn a_junction_into_the_skills_folder_is_not_deleted_through() {
    let (dir, store) = temp_store().await;
    let skills = dir.path().join("skills");
    let lib = service(&store, &new_bus(), &skills);
    lib.create_skill(DRAFT).await.unwrap();
    let link = dir.path().join("link");
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(skills.join("pr-tests"))
        .output()
        .unwrap();
    assert!(made.status.success());
    let item = lib
        .add(
            "linked",
            LibraryConfig::Skill(SkillConfig {
                path: link.to_string_lossy().into_owned(),
            }),
            None,
        )
        .await
        .unwrap();
    let list = lib.list().await.unwrap();
    assert!(
        !list
            .iter()
            .find(|e| e.item.name == "linked")
            .unwrap()
            .dots_folder
    );
    lib.remove(&item.id, false).await.unwrap();
    assert!(skills.join("pr-tests").join("SKILL.md").exists());
}
