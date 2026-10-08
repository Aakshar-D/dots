#![allow(dead_code)]

use dots_core::model::DotSpec;
use dots_core::store::Store;
use tempfile::TempDir;

pub async fn temp_store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("dots.db")).await.unwrap();
    (dir, store)
}

pub fn spec(name: &str) -> DotSpec {
    DotSpec::new(name, "Do the thing.", "C:/tmp/repo")
}
