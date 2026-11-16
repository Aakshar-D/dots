//! The app's side of what Open Sage does on the computer (`dots_core::open_dot::desk`): it
//! writes the note files, and opens sites, folders and apps the way this system does.

use std::path::{Path, PathBuf};
use std::process::Command;

use dots_core::open_dot::desk::{Desk, DeskApp, DeskTask, FAILED};
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

/// The folder in the person's Documents that notes are written to.
const NOTES_FOLDER: &str = "Sage Notes";
/// How many notes may share a title before the next is refused.
const SAME_TITLE_MOST: u32 = 200;

const NO_FOLDER: &str = "I can't find that folder on this computer.";
const NOT_WRITTEN: &str = "I couldn't write the note.";
#[cfg(not(any(windows, target_os = "macos")))]
const NO_APPS_HERE: &str = "I can't open apps on this computer yet.";

pub struct SystemDesk {
    app: AppHandle,
}

impl SystemDesk {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    fn notes_dir(&self) -> Result<PathBuf, String> {
        let documents = self
            .app
            .path()
            .document_dir()
            .map_err(|_| NO_FOLDER.to_string())?;
        Ok(documents.join(NOTES_FOLDER))
    }

    fn open_path(&self, path: &Path) -> Result<(), String> {
        self.app
            .opener()
            .open_path(path.to_string_lossy().to_string(), None::<&str>)
            .map_err(|e| {
                tracing::warn!("open sage: a path could not be opened: {e}");
                FAILED.to_string()
            })
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        self.app.opener().open_url(url, None::<&str>).map_err(|e| {
            tracing::warn!("open sage: an address could not be opened: {e}");
            FAILED.to_string()
        })
    }

    fn open_folder(&self, folder: tauri::Result<PathBuf>) -> Result<(), String> {
        let folder = folder.map_err(|_| NO_FOLDER.to_string())?;
        self.open_path(&folder)
    }

    fn open_app(&self, app: DeskApp) -> Result<(), String> {
        let path = self.app.path();
        match app {
            DeskApp::Documents => self.open_folder(path.document_dir()),
            DeskApp::Downloads => self.open_folder(path.download_dir()),
            DeskApp::Desktop => self.open_folder(path.desktop_dir()),
            DeskApp::Pictures => self.open_folder(path.picture_dir()),
            DeskApp::Notes => {
                let dir = self.notes_dir()?;
                std::fs::create_dir_all(&dir).map_err(|_| NO_FOLDER.to_string())?;
                self.open_path(&dir)
            }
            DeskApp::Browser => self.open_url("https://www.google.com/"),
            other => launch(other),
        }
    }
}

impl Desk for SystemDesk {
    fn carry_out(&self, task: &DeskTask) -> Result<(), String> {
        match task {
            DeskTask::Note { title, text, plain } => {
                let dir = self.notes_dir()?;
                let path = write_note(&dir, title, text, *plain).map_err(|e| {
                    tracing::warn!("open sage: a note could not be written: {e}");
                    NOT_WRITTEN.to_string()
                })?;
                self.open_path(&path)
            }
            DeskTask::Web { url, .. } | DeskTask::Search { url } => self.open_url(url),
            DeskTask::App { app } => self.open_app(*app),
        }
    }
}

/// Writes `text` to a new file named after `title` in `dir` (made when missing): `.txt` for
/// a plain note, `.md` otherwise. A file that is there is never written over: the next free
/// name is used ("Shopping 2"). The file written.
pub fn write_note(dir: &Path, title: &str, text: &str, plain: bool) -> std::io::Result<PathBuf> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let ending = if plain { "txt" } else { "md" };
    let mut text = text.to_string();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    for n in 1..=SAME_TITLE_MOST {
        let name = if n == 1 {
            format!("{title}.{ending}")
        } else {
            format!("{title} {n}.{ending}")
        };
        let path = dir.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(text.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "too many notes with this title",
    ))
}

fn started(command: &mut Command) -> Result<(), String> {
    command.spawn().map(|_| ()).map_err(|e| {
        tracing::warn!("open sage: an app could not be started: {e}");
        FAILED.to_string()
    })
}

/// Starts one of the apps of the fixed list. Each is a program of the system itself, named
/// here: nothing the model wrote is run.
#[cfg(windows)]
fn launch(app: DeskApp) -> Result<(), String> {
    match app {
        DeskApp::Notepad => started(&mut Command::new("notepad.exe")),
        DeskApp::Calculator => started(&mut Command::new("calc.exe")),
        DeskApp::Files => started(&mut Command::new("explorer.exe")),
        DeskApp::Paint => started(&mut Command::new("mspaint.exe")),
        DeskApp::SystemSettings => started(Command::new("explorer.exe").arg("ms-settings:")),
        // Windows Terminal where it is installed, the command prompt where it is not.
        DeskApp::Terminal => started(&mut Command::new("wt.exe"))
            .or_else(|_| started(Command::new("cmd.exe").args(["/C", "start", "cmd.exe"]))),
        _ => Err(FAILED.to_string()),
    }
}

#[cfg(target_os = "macos")]
fn launch(app: DeskApp) -> Result<(), String> {
    let name = match app {
        DeskApp::Notepad => "TextEdit",
        DeskApp::Calculator => "Calculator",
        DeskApp::Files => "Finder",
        DeskApp::Paint => "Preview",
        DeskApp::SystemSettings => "System Settings",
        DeskApp::Terminal => "Terminal",
        _ => return Err(FAILED.to_string()),
    };
    started(Command::new("open").args(["-a", name]))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn launch(_app: DeskApp) -> Result<(), String> {
    let _ = started;
    Err(NO_APPS_HERE.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_is_a_new_file_and_none_is_written_over() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("Sage Notes");
        let first = write_note(&notes, "Shopping", "milk", false).unwrap();
        assert_eq!(first, notes.join("Shopping.md"));
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "milk\n");
        let second = write_note(&notes, "Shopping", "eggs\n", false).unwrap();
        assert_eq!(second, notes.join("Shopping 2.md"));
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "milk\n");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "eggs\n");
        // A plain note is a text file of its own name.
        let plain = write_note(&notes, "Shopping", "tea", true).unwrap();
        assert_eq!(plain, notes.join("Shopping.txt"));
    }
}
