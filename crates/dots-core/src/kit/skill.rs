//! A skill folder: a `SKILL.md` and the files beside it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::frontmatter::parse_frontmatter;
use crate::{Error, Result};

/// Claude Code cuts a skill's `description` joined with `when_to_use` at this many characters in
/// its listing; the local engine's skills section does the same.
pub const LISTING_MAX_CHARS: usize = 1536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInfo {
    /// The frontmatter `name`, else the folder name. It may not be a valid Library name: the
    /// caller checks it with `check_name`.
    pub name: String,
    /// The frontmatter `description`, else the first non-empty line of the body (empty if none).
    pub description: String,
    pub when_to_use: Option<String>,
    pub dir: PathBuf,
    /// All scalar frontmatter fields (`allowed-tools` as a string included).
    pub fields: BTreeMap<String, String>,
}

impl SkillInfo {
    /// `description` and `when_to_use` joined, on one line and cut at `LISTING_MAX_CHARS`.
    pub fn listing(&self) -> String {
        let joined = match &self.when_to_use {
            Some(w) if !w.trim().is_empty() => format!("{} {}", self.description, w),
            _ => self.description.clone(),
        };
        let one_line = joined.split_whitespace().collect::<Vec<_>>().join(" ");
        one_line.chars().take(LISTING_MAX_CHARS).collect()
    }
}

fn folder_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Reads `<dir>/SKILL.md`. `NotFound` when the folder or the file is missing.
pub fn read_skill(dir: &Path) -> Result<SkillInfo> {
    if !dir.is_dir() {
        return Err(Error::NotFound(format!(
            "folder not found: {}",
            dir.display()
        )));
    }
    let file = dir.join("SKILL.md");
    let bytes = fs::read(&file).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            Error::NotFound(format!("SKILL.md not found in {}", dir.display()))
        }
        _ => Error::Io(e),
    })?;
    let parsed = parse_frontmatter(&String::from_utf8_lossy(&bytes));
    let field = |key: &str| {
        parsed
            .fields
            .get(key)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let description = field("description")
        .or_else(|| {
            parsed
                .body
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(String::from)
        })
        .unwrap_or_default();
    Ok(SkillInfo {
        name: field("name").unwrap_or_else(|| folder_name(dir)),
        description,
        when_to_use: field("when_to_use"),
        dir: dir.to_path_buf(),
        fields: parsed.fields,
    })
}

fn skills_in_children(root: &Path) -> Vec<SkillInfo> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .filter(|p| p.join("SKILL.md").is_file())
        .collect();
    dirs.sort();
    dirs.iter().filter_map(|d| read_skill(d).ok()).collect()
}

/// The skills under `root` (`library_scan_skills`): `root` itself when it holds a `SKILL.md`,
/// else its subfolders that do, else those of `root/skills`. By folder, so by path. `NotFound`
/// when `root` is not a folder.
pub fn scan_skills(root: &Path) -> Result<Vec<SkillInfo>> {
    if !root.is_dir() {
        return Err(Error::NotFound(format!(
            "folder not found: {}",
            root.display()
        )));
    }
    if root.join("SKILL.md").is_file() {
        return Ok(vec![read_skill(root)?]);
    }
    let found = skills_in_children(root);
    if found.is_empty() {
        return Ok(skills_in_children(&root.join("skills")));
    }
    Ok(found)
}
