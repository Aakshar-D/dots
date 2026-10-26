mod common;

use common::{spec, temp_store};
use dots_core::store::{backup_database, Store};

#[tokio::test]
async fn a_backup_holds_what_was_written_also_after_the_store_closed() {
    let (dir, store) = temp_store().await;
    let dot = store.create_dot(&spec("digest"), None).await.unwrap();
    store.close().await;
    let dest = dir.path().join("backups").join("copy.db");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    backup_database(&dir.path().join("dots.db"), &dest)
        .await
        .unwrap();
    let copy = Store::open(&dest).await.unwrap();
    let dots = copy.list_dots().await.unwrap();
    assert_eq!(dots.len(), 1);
    assert_eq!(dots[0].id, dot.id);
}

#[tokio::test]
async fn a_backup_never_overwrites_a_file() {
    let (dir, store) = temp_store().await;
    store.close().await;
    let dest = dir.path().join("copy.db");
    std::fs::write(&dest, b"keep me").unwrap();
    assert!(backup_database(&dir.path().join("dots.db"), &dest)
        .await
        .is_err());
    assert_eq!(std::fs::read(&dest).unwrap(), b"keep me");
}

#[tokio::test]
async fn a_missing_database_is_an_error_and_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("dots.db");
    let dest = dir.path().join("copy.db");
    assert!(backup_database(&db, &dest).await.is_err());
    assert!(!db.exists());
    assert!(!dest.exists());
}
