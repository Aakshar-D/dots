//! A dot's making settings and who made it (dots-make-dots design sections 3, 5, 6 and 9): the
//! switch is off for every new dot and its two limits have ranges, a made dot records its maker
//! and keeps the record through the maker's rename and delete, and a made dot is stored with
//! exactly the variables it was given.

mod common;

use common::{spec, temp_store};
use dots_core::library::{LibraryConfig, McpServerConfig};
use dots_core::model::{
    Dot, DotSpec, Made, Making, MAKE_PER_RUN_DEFAULT, MAKE_PER_RUN_MAX, MAKE_PER_RUN_RULE,
    MAKE_TOTAL_DEFAULT, MAKE_TOTAL_MAX, MAKE_TOTAL_RULE,
};
use dots_core::store::Store;
use dots_core::vault::Grants;
use dots_core::Error;

fn grants(pairs: &[(&str, &[&str])]) -> Grants {
    Grants(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
            .collect(),
    )
}

/// The record of a dot `maker` made in its run `run-1`.
fn made_by(maker: &Dot) -> Made {
    Made {
        by_dot_id: Some(maker.id.clone()),
        by_name: maker.spec.name.clone(),
        run_id: "run-1".into(),
        purpose: "Watches three blogs each Monday.".into(),
    }
}

async fn maker(store: &Store) -> Dot {
    let mut s = spec("Researcher");
    s.making.on = true;
    store.create_dot(&s, None, "default").await.unwrap()
}

/// Puts an MCP server named `github` in the Library whose config reads `GITHUB_TOKEN`.
async fn add_github(store: &Store) {
    let config = LibraryConfig::McpServer(McpServerConfig::Stdio {
        command: "node".into(),
        args: Vec::new(),
        cwd: None,
        env: [("TOKEN".to_string(), "${GITHUB_TOKEN}".to_string())].into(),
    });
    store
        .add_library_item("github", &config, None)
        .await
        .unwrap();
}

fn invalid_text<T: std::fmt::Debug>(res: Result<T, Error>) -> String {
    match res {
        Err(Error::Invalid(text)) => text,
        other => panic!("expected Error::Invalid, got {other:?}"),
    }
}

#[tokio::test]
async fn a_new_dot_cannot_make_dots() {
    let off = Making {
        on: false,
        per_run: 3,
        total: 10,
    };
    assert_eq!((MAKE_PER_RUN_DEFAULT, MAKE_TOTAL_DEFAULT), (3, 10));
    assert_eq!((MAKE_PER_RUN_MAX, MAKE_TOTAL_MAX), (10, 50));
    assert_eq!(Making::default(), off);
    assert_eq!(spec("triage").making, off);

    let (_dir, store) = temp_store().await;
    let dot = store
        .create_dot(&spec("triage"), None, "default")
        .await
        .unwrap();
    assert_eq!(dot.spec.making, off);
    assert_eq!(dot.made, None);
    let read = store.get_dot(&dot.id).await.unwrap();
    assert_eq!(read.spec.making, off);
    assert_eq!(read.made, None);
}

#[tokio::test]
async fn making_round_trips() {
    let (_dir, store) = temp_store().await;
    let mut s = spec("Researcher");
    s.making = Making {
        on: true,
        per_run: 5,
        total: 20,
    };
    let dot = store.create_dot(&s, None, "default").await.unwrap();
    assert_eq!(store.get_dot(&dot.id).await.unwrap().spec.making, s.making);

    s.making = Making {
        on: false,
        per_run: 1,
        total: 50,
    };
    store.update_dot(&dot.id, &s).await.unwrap();
    assert_eq!(store.get_dot(&dot.id).await.unwrap().spec.making, s.making);
}

#[tokio::test]
async fn making_out_of_range_is_refused() {
    assert_eq!(MAKE_PER_RUN_RULE, "A dot may make 1 to 10 dots in one run.");
    assert_eq!(MAKE_TOTAL_RULE, "A dot may keep 1 to 50 made dots.");
    let (_dir, store) = temp_store().await;
    let cases = [
        (0, 10, MAKE_PER_RUN_RULE),
        (11, 10, MAKE_PER_RUN_RULE),
        (3, 0, MAKE_TOTAL_RULE),
        (3, 51, MAKE_TOTAL_RULE),
    ];
    // The ranges hold whether or not the switch is on.
    for on in [false, true] {
        for (per_run, total, rule) in cases {
            let making = Making { on, per_run, total };
            assert_eq!(invalid_text(making.validate()), rule, "{making:?}");
            let mut s = spec("triage");
            s.making = making;
            assert_eq!(invalid_text(s.validate()), rule, "{making:?}");
            assert_eq!(
                invalid_text(store.create_dot(&s, None, "default").await),
                rule,
                "{making:?}"
            );
        }
    }
    assert!(store.list_dots().await.unwrap().is_empty());
    for (per_run, total) in [(1, 1), (10, 50)] {
        Making {
            on: true,
            per_run,
            total,
        }
        .validate()
        .unwrap();
    }
}

#[test]
fn a_spec_without_making_reads_as_off() {
    let mut s = spec("triage");
    s.making = Making {
        on: true,
        per_run: 5,
        total: 20,
    };
    let mut json = serde_json::to_value(&s).unwrap();
    assert!(json.as_object_mut().unwrap().remove("making").is_some());
    let read: DotSpec = serde_json::from_value(json).unwrap();
    assert_eq!(read.making, Making::default());

    // A part of it that is missing takes its default too.
    let part: Making = serde_json::from_str(r#"{"on":true}"#).unwrap();
    assert_eq!(
        part,
        Making {
            on: true,
            per_run: 3,
            total: 10
        }
    );
}

#[tokio::test]
async fn a_made_dot_records_its_maker() {
    let (_dir, store) = temp_store().await;
    let maker = maker(&store).await;
    let by_hand = store
        .create_dot(&spec("By hand"), None, "default")
        .await
        .unwrap();
    assert_eq!(store.count_made_dots(&maker.id).await.unwrap(), 0);
    assert!(store.list_made_dots(&maker.id).await.unwrap().is_empty());

    let made = made_by(&maker);
    let zed = store
        .create_made_dot(&spec("Zed Watcher"), None, "default", &made)
        .await
        .unwrap();
    assert_eq!(zed.made, Some(made.clone()));
    assert_eq!(store.get_dot(&zed.id).await.unwrap().made, Some(made));
    // A made dot cannot make dots unless its spec says so.
    assert_eq!(zed.spec.making, Making::default());

    let blog = store
        .create_made_dot(&spec("Blog Watcher"), None, "default", &made_by(&maker))
        .await
        .unwrap();
    let listed = store.list_made_dots(&maker.id).await.unwrap();
    // By name; the dot a person made is not among them.
    assert_eq!(
        listed.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
        [blog.id.as_str(), zed.id.as_str()]
    );
    assert_eq!(store.count_made_dots(&maker.id).await.unwrap(), 2);
    assert_eq!(store.count_made_dots(&by_hand.id).await.unwrap(), 0);
    assert!(store.list_made_dots(&zed.id).await.unwrap().is_empty());
    assert_eq!(store.get_dot(&by_hand.id).await.unwrap().made, None);

    // A made dot follows the rules of any dot: its name is taken once.
    let again = store
        .create_made_dot(&spec("blog watcher"), None, "default", &made_by(&maker))
        .await;
    assert!(matches!(again, Err(Error::Conflict(_))), "{again:?}");
    assert_eq!(store.count_made_dots(&maker.id).await.unwrap(), 2);
}

#[tokio::test]
async fn a_made_dot_keeps_the_variables_it_was_given() {
    let (_dir, store) = temp_store().await;
    add_github(&store).await;
    let maker = maker(&store).await;
    let token = grants(&[("mcp:github", &["GITHUB_TOKEN"])]);

    // A person's save grants a newly used server the variables of its config.
    let mut by_hand = spec("By hand");
    by_hand.uses.mcp_servers = vec!["github".into()];
    let saved = store.create_dot(&by_hand, None, "default").await.unwrap();
    assert_eq!(saved.spec.variables, token);

    // A made dot is stored with what it was given: here nothing.
    let mut s = spec("Blog Watcher");
    s.uses.mcp_servers = vec!["github".into()];
    assert_eq!(s.variables, Grants::default());
    let dot = store
        .create_made_dot(&s, None, "default", &made_by(&maker))
        .await
        .unwrap();
    assert_eq!(dot.spec.variables, Grants::default());
    assert_eq!(
        store.get_dot(&dot.id).await.unwrap().spec.variables,
        Grants::default()
    );
    // And with what it was given when that is something.
    let mut given = spec("Issue Watcher");
    given.uses.mcp_servers = vec!["github".into()];
    given.variables = token.clone();
    let dot_given = store
        .create_made_dot(&given, None, "default", &made_by(&maker))
        .await
        .unwrap();
    assert_eq!(dot_given.spec.variables, token);

    // The same for a change: a dot that starts to use the server.
    let plain = store
        .create_made_dot(&spec("Plain"), None, "default", &made_by(&maker))
        .await
        .unwrap();
    let mut uses_it = plain.spec.clone();
    uses_it.uses.mcp_servers = vec!["github".into()];
    let exact = store.update_dot_exact(&plain.id, &uses_it).await.unwrap();
    assert_eq!(exact.spec.uses.mcp_servers, ["github"]);
    assert_eq!(exact.spec.variables, Grants::default());
    assert_eq!(exact.made, plain.made);

    let other = store
        .create_made_dot(&spec("Other"), None, "default", &made_by(&maker))
        .await
        .unwrap();
    let mut uses_it = other.spec.clone();
    uses_it.uses.mcp_servers = vec!["github".into()];
    let synced = store.update_dot(&other.id, &uses_it).await.unwrap();
    assert_eq!(synced.spec.variables, token);
}

#[tokio::test]
async fn renaming_the_maker_renames_it_on_its_made_dots() {
    let (_dir, store) = temp_store().await;
    let maker = maker(&store).await;
    let mut other_spec = spec("Planner");
    other_spec.making.on = true;
    let other = store
        .create_dot(&other_spec, None, "default")
        .await
        .unwrap();
    let mine = store
        .create_made_dot(&spec("Blog Watcher"), None, "default", &made_by(&maker))
        .await
        .unwrap();
    let theirs = store
        .create_made_dot(&spec("Plan Keeper"), None, "default", &made_by(&other))
        .await
        .unwrap();

    // A save that keeps the name changes nothing on the made dots.
    let mut s = maker.spec.clone();
    s.instructions = "Research with care.".into();
    store.update_dot(&maker.id, &s).await.unwrap();
    assert_eq!(store.get_dot(&mine.id).await.unwrap().made, mine.made);

    s.name = "Scout".into();
    store.update_dot(&maker.id, &s).await.unwrap();
    let made = store.get_dot(&mine.id).await.unwrap().made.unwrap();
    assert_eq!(made.by_name, "Scout");
    assert_eq!(made.by_dot_id.as_deref(), Some(maker.id.as_str()));
    assert_eq!(made.run_id, "run-1");
    assert_eq!(made.purpose, "Watches three blogs each Monday.");
    // Another maker's made dot keeps its maker's name.
    assert_eq!(store.get_dot(&theirs.id).await.unwrap().made, theirs.made);

    // The exact save renames as well.
    s.name = "Pathfinder".into();
    store.update_dot_exact(&maker.id, &s).await.unwrap();
    let made = store.get_dot(&mine.id).await.unwrap().made.unwrap();
    assert_eq!(made.by_name, "Pathfinder");

    // A rename that is refused renames nothing.
    s.name = "planner".into();
    let clash = store.update_dot(&maker.id, &s).await;
    assert!(matches!(clash, Err(Error::Conflict(_))), "{clash:?}");
    let made = store.get_dot(&mine.id).await.unwrap().made.unwrap();
    assert_eq!(made.by_name, "Pathfinder");
}

#[tokio::test]
async fn deleting_the_maker_keeps_its_made_dots() {
    let (_dir, store) = temp_store().await;
    let maker = maker(&store).await;
    let dot = store
        .create_made_dot(&spec("Blog Watcher"), None, "default", &made_by(&maker))
        .await
        .unwrap();
    let mut renamed = maker.spec.clone();
    renamed.name = "Scout".into();
    store.update_dot(&maker.id, &renamed).await.unwrap();

    store.delete_dot(&maker.id).await.unwrap();
    assert!(matches!(
        store.get_dot(&maker.id).await,
        Err(Error::NotFound(_))
    ));
    let kept = store.get_dot(&dot.id).await.unwrap();
    assert_eq!(
        kept.made,
        Some(Made {
            by_dot_id: None,
            by_name: "Scout".into(),
            run_id: "run-1".into(),
            purpose: "Watches three blogs each Monday.".into(),
        })
    );
    assert_eq!(kept.spec, dot.spec);
    // It is no longer counted for the maker that is gone.
    assert_eq!(store.count_made_dots(&maker.id).await.unwrap(), 0);
    assert!(store.list_made_dots(&maker.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn saving_a_made_dot_keeps_who_made_it() {
    let (_dir, store) = temp_store().await;
    let maker = maker(&store).await;
    let dot = store
        .create_made_dot(&spec("Blog Watcher"), None, "default", &made_by(&maker))
        .await
        .unwrap();
    let mut s = dot.spec.clone();
    s.name = "Feed Watcher".into();
    s.instructions = "Watch the feeds.".into();
    s.making.on = true;
    let saved = store.update_dot(&dot.id, &s).await.unwrap();
    assert_eq!(saved.made, dot.made);
    assert_eq!(saved.spec.name, "Feed Watcher");
    assert!(saved.spec.making.on);
    let saved = store.update_dot_exact(&dot.id, &dot.spec).await.unwrap();
    assert_eq!(saved.made, dot.made);
    assert_eq!(store.count_made_dots(&maker.id).await.unwrap(), 1);
}
