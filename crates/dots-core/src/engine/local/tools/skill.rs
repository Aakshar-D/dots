//! The `skill` tool (policy name `Skill`) and the system prompt's list of skills (MCP/skills
//! spec section 7). A skill is read from the folder the Library names, never from the
//! workspace: without `file` the tool returns the `SKILL.md` body, `${CLAUDE_SKILL_DIR}`
//! replaced by the skill's folder, and the list of the skill's files; with `file` it returns
//! that file, which must lie inside the skill's folder. A skill's `allowed-tools` is ignored:
//! its scripts run only through `shell`, under the dot's policy.

use std::path::{Component, Path};

use serde_json::{json, Value};

use super::{cap_head, str_arg, tool, ToolOutput, MAX_ENTRIES, MAX_READ_BYTES};
use crate::kit::parse_frontmatter;
use crate::run_tools::RunSkill;

/// The first line of the system prompt's skills section.
const PROMPT_HEAD: &str =
    "Skills you can use. Call the tool `skill` with a name to read one before you follow it.";

/// The function definition, offered to a run that has skills.
pub fn schema() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
    tool(
        "skill",
        "Read a skill: its instructions and the list of its files, or one of its files.",
        json!({
            "name": s("The skill's name, as the system prompt lists it"),
            "file": s("A file of the skill, relative to its folder; default: its instructions")
        }),
        &["name"],
    )
}

/// The system prompt's section that lists `skills`, one `name: description` line each; empty
/// without skills.
pub fn prompt_section(skills: &[RunSkill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = PROMPT_HEAD.to_string();
    for skill in skills {
        out.push_str(&format!("\n- {}: {}", skill.name, skill.info.listing()));
    }
    out
}

/// Runs an allowed `skill` call (its input as `prepare` left it) against the run's skills.
pub async fn run(skills: &[RunSkill], input: &Value) -> ToolOutput {
    let name = str_arg(input, "name");
    let Some(skill) = skills.iter().find(|s| s.name == name) else {
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        return ToolOutput::err(if names.is_empty() {
            format!("unknown skill '{name}'. This run has no skills.")
        } else {
            format!("unknown skill '{name}'. Skills: {}", names.join(", "))
        });
    };
    let dir = skill.info.dir.clone();
    // An empty `file` asks for the instructions, as no `file` does.
    let file = input
        .get("file")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(str::to_string);
    let read = tokio::task::spawn_blocking(move || match file {
        Some(file) => read_file(&dir, &file),
        None => instructions(&dir),
    })
    .await;
    match read {
        Ok(Ok(text)) => ToolOutput::ok(cap_head(text)),
        Ok(Err(e)) => ToolOutput::err(e),
        Err(e) => ToolOutput::err(format!("reading the skill failed: {e}")),
    }
}

/// The `SKILL.md` body with the folder put in, then the skill's other files.
fn instructions(dir: &Path) -> Result<String, String> {
    let text = read_text(&dir.join("SKILL.md"), "SKILL.md")?;
    let body = parse_frontmatter(&text)
        .body
        .replace("${CLAUDE_SKILL_DIR}", &dir.to_string_lossy());
    let mut out = body.trim().to_string();
    let (files, more) = list_files(dir);
    if !files.is_empty() {
        out.push_str("\n\nFiles in this skill (read one with `file`):");
        for file in files {
            out.push_str(&format!("\n- {file}"));
        }
        if more {
            out.push_str(&format!("\n(only the first {MAX_ENTRIES} are listed)"));
        }
    }
    Ok(out)
}

/// The skill's files below `dir`, relative with `/`, sorted, without `SKILL.md`, hidden
/// entries (`.git`) and links; true when there were more than `MAX_ENTRIES`.
fn list_files(dir: &Path) -> (Vec<String>, bool) {
    let mut files = Vec::new();
    let mut folders = vec![(dir.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if name.starts_with('.') || kind.is_symlink() {
                continue;
            }
            let rel = format!("{prefix}{name}");
            if kind.is_dir() {
                folders.push((entry.path(), format!("{rel}/")));
            } else if rel != "SKILL.md" {
                files.push(rel);
            }
        }
    }
    files.sort();
    let more = files.len() > MAX_ENTRIES;
    files.truncate(MAX_ENTRIES);
    (files, more)
}

/// The file `file` of the skill in `dir`. It must name a file inside the folder: a path that
/// is absolute, holds `..`, or leads out of the folder through a link is refused.
fn read_file(dir: &Path, file: &str) -> Result<String, String> {
    let outside = || format!("{file} is not inside the skill folder");
    let given = Path::new(file);
    let plain = given.components().all(|c| match c {
        Component::Normal(n) => !n.to_string_lossy().contains(':'),
        Component::CurDir => true,
        _ => false,
    });
    if !plain {
        return Err(outside());
    }
    let root =
        std::fs::canonicalize(dir).map_err(|e| format!("the skill folder cannot be read: {e}"))?;
    let path = std::fs::canonicalize(root.join(given))
        .map_err(|_| format!("{file} was not found in the skill folder"))?;
    if !path.starts_with(&root) {
        return Err(outside());
    }
    if path.is_dir() {
        return Err(format!("{file} is a folder; name a file in it"));
    }
    read_text(&path, file)
}

/// A text file, refused above `MAX_READ_BYTES`; bytes that are not UTF-8 are replaced.
fn read_text(path: &Path, shown: &str) -> Result<String, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{shown} could not be read: {e}"))?
        .len();
    if size > MAX_READ_BYTES {
        return Err(format!("{shown} is larger than 10 MB"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{shown} could not be read: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
