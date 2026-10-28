//! The apps that come with Dots (app library spec section 8): Projects, whose files are built into
//! Dots from `crates/dots-core/apps/projects/`. A file added to that folder is listed in
//! `PROJECTS_FILES` too; a test checks that the two agree.

use std::collections::BTreeMap;

use super::source::{self, AppSource};
use crate::Result;

/// The name of the Projects app.
pub const PROJECTS: &str = "projects";

/// Projects' files, by their path in the app folder.
const PROJECTS_FILES: &[(&str, &[u8])] = &[
    ("README.md", include_bytes!("../../apps/projects/README.md")),
    ("app.json", include_bytes!("../../apps/projects/app.json")),
    ("logic.js", include_bytes!("../../apps/projects/logic.js")),
    (
        "migrations/0001_projects.sql",
        include_bytes!("../../apps/projects/migrations/0001_projects.sql"),
    ),
    (
        "sample.sql",
        include_bytes!("../../apps/projects/sample.sql"),
    ),
    (
        "tests/assign.test.js",
        include_bytes!("../../apps/projects/tests/assign.test.js"),
    ),
    (
        "tests/tasks.test.js",
        include_bytes!("../../apps/projects/tests/tasks.test.js"),
    ),
    (
        "tests/undo.test.js",
        include_bytes!("../../apps/projects/tests/undo.test.js"),
    ),
    (
        "ui/index.html",
        include_bytes!("../../apps/projects/ui/index.html"),
    ),
];

/// The Dots release the bundled apps come with: `AppOrigin::Bundled`'s `release`.
pub fn release() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Projects' files as Dots holds them.
pub fn projects_files() -> BTreeMap<String, Vec<u8>> {
    PROJECTS_FILES
        .iter()
        .map(|(path, bytes)| (path.to_string(), bytes.to_vec()))
        .collect()
}

/// Projects, read and checked as a proposal's folder is (`source::read_files`). Blocks.
pub fn projects() -> Result<AppSource> {
    source::read_files(&projects_files())
}
