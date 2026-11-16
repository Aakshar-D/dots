//! The app's side of what Open Sage does on the computer (`dots_core::open_dot::desk`): it
//! writes the note files, and opens sites, folders and apps the way this system does.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dots_core::open_dot::desk::{
    Desk, DeskApp, DeskKey, DeskTask, SettingsPage, COMMAND_STOPPED, FAILED,
};
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

/// The folder in the person's Documents that notes are written to.
const NOTES_FOLDER: &str = "Sage Notes";
/// How many notes may share a title before the next is refused.
const SAME_TITLE_MOST: u32 = 200;

/// The longest a command may run before it is stopped.
const COMMAND_LIMIT: Duration = Duration::from_secs(60);
/// The most of what a command printed that is kept.
const OUTPUT_BYTES: usize = 4000;
const TOO_LONG: &str = "That took more than a minute, so I stopped it.";
const NOT_OPEN: &str = "That one isn't open.";

const NO_FOLDER: &str = "I can't find that folder on this computer.";
const NOT_WRITTEN: &str = "I couldn't write the note.";
const NOTE_NOT_OPENED: &str =
    "The note is written, in your Sage Notes folder, but I couldn't open it.";
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
    fn carry_out(&self, task: &DeskTask, stop: &AtomicBool) -> Result<String, String> {
        match task {
            DeskTask::Command { command } => return run_command(command, stop),
            DeskTask::Close { app } => close(*app),
            DeskTask::Note { title, text, plain } => {
                let dir = self.notes_dir()?;
                let path = write_note(&dir, title, text, *plain).map_err(|e| {
                    tracing::warn!("open sage: a note could not be written: {e}");
                    NOT_WRITTEN.to_string()
                })?;
                self.open_path(&path)
                    .map_err(|_| NOTE_NOT_OPENED.to_string())
            }
            DeskTask::Web { url, .. } | DeskTask::Search { url } => self.open_url(url),
            DeskTask::App { app } => self.open_app(*app),
            DeskTask::Page { page } => open_page(*page),
            DeskTask::Named { name } => open_named(name),
            DeskTask::Key { key } => press(*key),
        }
        .map(|()| String::new())
    }
}

/// A program started with no window of its own.
fn quiet(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW
        command.creation_flags(0x0800_0000);
    }
    command
}

/// The system's shell, given one command.
#[cfg(windows)]
fn shell(command: &str) -> Command {
    let mut shell = Command::new("powershell.exe");
    shell.args(["-NoProfile", "-NonInteractive", "-Command", command]);
    shell
}

#[cfg(target_os = "macos")]
fn shell(command: &str) -> Command {
    let mut shell = Command::new("/bin/zsh");
    shell.args(["-c", command]);
    shell
}

#[cfg(not(any(windows, target_os = "macos")))]
fn shell(command: &str) -> Command {
    let mut shell = Command::new("/bin/sh");
    shell.args(["-c", command]);
    shell
}

/// Ends the process `pid` and what it started.
fn end(pid: u32) {
    #[cfg(windows)]
    let _ = quiet(Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"])).output();
    #[cfg(not(windows))]
    let _ = Command::new("kill").args(["-9", &pid.to_string()]).output();
}

/// `text` cut to `OUTPUT_BYTES`, on a character.
fn kept(text: &str) -> String {
    let mut end = text.len().min(OUTPUT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].trim().to_string()
}

/// The first `OUTPUT_BYTES` of the file at `path`, as text; the file is removed.
fn printed(path: &Path) -> String {
    use std::io::Read;
    let mut bytes = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(OUTPUT_BYTES as u64).read_to_end(&mut bytes);
    }
    let _ = std::fs::remove_file(path);
    kept(&String::from_utf8_lossy(&bytes))
}

/// Runs one command the person said yes to. What it printed (its errors after its output),
/// or a sentence saying that it failed, with the first line of its error, or took too long.
///
/// Its output goes to two files, not to pipes: a program the command starts and leaves
/// running would hold a pipe open, and the command would seem never to end. So the wait is
/// for the shell itself.
///
/// It is ended, with what it started, soon after `stop` is set.
pub fn run_command(command: &str, stop: &AtomicBool) -> Result<String, String> {
    let failed = |e: std::io::Error| {
        tracing::warn!("open sage: a command could not be run: {e}");
        FAILED.to_string()
    };
    let id = format!("sage-command-{}-{}", std::process::id(), {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    });
    let out_path = std::env::temp_dir().join(format!("{id}.out"));
    let err_path = std::env::temp_dir().join(format!("{id}.err"));
    let out = std::fs::File::create(&out_path).map_err(failed)?;
    let err = std::fs::File::create(&err_path).map_err(failed)?;
    let mut shell = shell(command);
    shell.stdin(Stdio::null()).stdout(out).stderr(err);
    let mut child = quiet(&mut shell).spawn().map_err(failed)?;
    let started = std::time::Instant::now();
    let mut stopped = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if stop.load(Ordering::SeqCst) => {
                end(child.id());
                let _ = child.wait();
                stopped = true;
                break None;
            }
            Ok(None) if started.elapsed() < COMMAND_LIMIT => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                end(child.id());
                let _ = child.wait();
                break None;
            }
            Err(e) => {
                let _ = std::fs::remove_file(&out_path);
                let _ = std::fs::remove_file(&err_path);
                return Err(failed(e));
            }
        }
    };
    let (output, error) = (printed(&out_path), printed(&err_path));
    match status {
        None if stopped => Err(COMMAND_STOPPED.to_string()),
        None => Err(TOO_LONG.to_string()),
        // An error the shell went on after is shown with what was printed.
        Some(status) if status.success() => Ok(format!("{output}\n{error}").trim().to_string()),
        Some(_) => {
            let first = error
                .lines()
                .chain(output.lines())
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or("");
            let first: String = first.chars().take(160).collect();
            Err(format!("That failed. {first}").trim().to_string())
        }
    }
}

/// Asks an open app to close, as its close button does: it may ask about unsaved work.
#[cfg(windows)]
fn close(app: DeskApp) -> Result<(), String> {
    let images: &[&str] = match app {
        DeskApp::Notepad => &["notepad.exe"],
        DeskApp::Calculator => &["CalculatorApp.exe", "calc.exe"],
        DeskApp::Paint => &["mspaint.exe"],
        DeskApp::SystemSettings => &["SystemSettings.exe"],
        _ => return Err(FAILED.to_string()),
    };
    // Without /F: the app gets the same request its close button sends.
    let closed = images.iter().any(|image| {
        quiet(Command::new("taskkill").args(["/IM", image]))
            .output()
            .is_ok_and(|out| out.status.success())
    });
    if closed {
        Ok(())
    } else {
        Err(NOT_OPEN.to_string())
    }
}

#[cfg(target_os = "macos")]
fn close(app: DeskApp) -> Result<(), String> {
    let name = match app {
        DeskApp::Notepad => "TextEdit",
        DeskApp::Calculator => "Calculator",
        DeskApp::Paint => "Preview",
        DeskApp::SystemSettings => "System Settings",
        _ => return Err(FAILED.to_string()),
    };
    let asked = Command::new("osascript")
        .args(["-e", &format!("quit app \"{name}\"")])
        .output()
        .is_ok_and(|out| out.status.success());
    if asked {
        Ok(())
    } else {
        Err(NOT_OPEN.to_string())
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn close(_app: DeskApp) -> Result<(), String> {
    let _ = NOT_OPEN;
    Err(NO_APPS_HERE.to_string())
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

/// Said when no installed app has the name asked for.
fn no_such_app(name: &str) -> String {
    format!("I can't find an app called {name} on this computer.")
}

/// Opens one page of the computer's settings, by its address written in the core.
#[cfg(windows)]
fn open_page(page: SettingsPage) -> Result<(), String> {
    started(Command::new("explorer.exe").arg(page.windows()))
}

#[cfg(target_os = "macos")]
fn open_page(page: SettingsPage) -> Result<(), String> {
    started(Command::new("open").arg(page.mac()))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn open_page(_page: SettingsPage) -> Result<(), String> {
    Err(NO_APPS_HERE.to_string())
}

/// The apps the Start menu lists, as (name, id). The command is written here; the name asked
/// for is no part of it.
#[cfg(windows)]
fn start_apps() -> Vec<(String, String)> {
    let out = quiet(Command::new("powershell.exe").args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8; \
         @(Get-StartApps | Select-Object Name, AppID) | ConvertTo-Json -Compress",
    ]))
    .stdin(Stdio::null())
    .output();
    let Ok(out) = out else {
        return Vec::new();
    };
    let listed: serde_json::Value =
        serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    listed
        .as_array()
        .map(|apps| {
            apps.iter()
                .filter_map(|app| {
                    Some((
                        app.get("Name")?.as_str()?.to_string(),
                        app.get("AppID")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Opens the installed app `name` means: the Start menu's own entry for it is started, by the
/// id the system gave. The name is only compared with the names listed.
#[cfg(windows)]
fn open_named(name: &str) -> Result<(), String> {
    let apps = start_apps();
    let names: Vec<String> = apps.iter().map(|(name, _)| name.clone()).collect();
    let found =
        dots_core::open_dot::desk::pick_app(name, &names).ok_or_else(|| no_such_app(name))?;
    let id = apps
        .iter()
        .find(|(listed, _)| listed == found)
        .map(|(_, id)| id.clone())
        .ok_or_else(|| no_such_app(name))?;
    started(Command::new("explorer.exe").arg(format!("shell:AppsFolder\\{id}")))
}

#[cfg(target_os = "macos")]
fn open_named(name: &str) -> Result<(), String> {
    // `open -a` takes the name as one argument and looks it up among the installed apps.
    let opened = Command::new("open")
        .args(["-a", name])
        .stdin(Stdio::null())
        .output()
        .is_ok_and(|out| out.status.success());
    if opened {
        Ok(())
    } else {
        Err(no_such_app(name))
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn open_named(_name: &str) -> Result<(), String> {
    Err(NO_APPS_HERE.to_string())
}

/// Presses a volume or media key, as the keyboard's own key does. The key codes and the
/// count are written here.
#[cfg(windows)]
fn press(key: DeskKey) -> Result<(), String> {
    // VK_VOLUME_UP, VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_NEXT_TRACK,
    // VK_MEDIA_PREV_TRACK. One press of a volume key is two parts in a hundred.
    let (code, times) = match key {
        DeskKey::VolumeUp => (0xAF, 5),
        DeskKey::VolumeDown => (0xAE, 5),
        DeskKey::Mute => (0xAD, 1),
        DeskKey::PlayPause => (0xB3, 1),
        DeskKey::Next => (0xB0, 1),
        DeskKey::Previous => (0xB1, 1),
    };
    let script = format!(
        "$k = Add-Type -PassThru -Namespace SageKeys -Name Press -MemberDefinition \
         '[DllImport(\"user32.dll\")] public static extern void keybd_event(byte v, byte s, uint f, System.UIntPtr e);'; \
         1..{times} | ForEach-Object {{ $k::keybd_event({code}, 0, 0, [UIntPtr]::Zero); \
         $k::keybd_event({code}, 0, 2, [UIntPtr]::Zero) }}"
    );
    let pressed = quiet(&mut shell(&script))
        .stdin(Stdio::null())
        .output()
        .is_ok_and(|out| out.status.success());
    if pressed {
        Ok(())
    } else {
        tracing::warn!("open sage: a key could not be pressed");
        Err(FAILED.to_string())
    }
}

#[cfg(target_os = "macos")]
fn press(key: DeskKey) -> Result<(), String> {
    let script = match key {
        DeskKey::VolumeUp => {
            "set volume output volume ((output volume of (get volume settings)) + 10)"
        }
        DeskKey::VolumeDown => {
            "set volume output volume ((output volume of (get volume settings)) - 10)"
        }
        DeskKey::Mute => "set volume output muted (not (output muted of (get volume settings)))",
        DeskKey::PlayPause | DeskKey::Next | DeskKey::Previous => {
            return Err("I can't press the music keys on a Mac yet.".to_string())
        }
    };
    let done = Command::new("osascript")
        .args(["-e", script])
        .stdin(Stdio::null())
        .output()
        .is_ok_and(|out| out.status.success());
    if done {
        Ok(())
    } else {
        Err(FAILED.to_string())
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn press(_key: DeskKey) -> Result<(), String> {
    Err(NO_APPS_HERE.to_string())
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
    fn a_command_gives_what_it_printed_or_says_how_it_failed() {
        let go = AtomicBool::new(false);
        let run_command = |command: &str| run_command(command, &go);
        assert_eq!(run_command("echo hello").unwrap(), "hello");
        // A program the command starts and leaves running does not hold it up.
        #[cfg(windows)]
        {
            let started = std::time::Instant::now();
            run_command(
                "Start-Process cmd.exe -ArgumentList '/c','timeout','/t','20' -WindowStyle Hidden",
            )
            .unwrap();
            assert!(started.elapsed() < Duration::from_secs(15));
        }
        let failed = run_command("exit 3").unwrap_err();
        assert!(failed.starts_with("That failed."), "{failed}");
        #[cfg(windows)]
        {
            let failed = run_command("Get-Item C:\\no\\such\\thing -ErrorAction Stop").unwrap_err();
            assert!(
                failed.starts_with("That failed. ") && failed.len() > 13,
                "{failed}"
            );
        }
    }

    #[test]
    fn a_command_told_to_stop_ends_at_once() {
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let told = stop.clone();
        let teller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            told.store(true, Ordering::SeqCst);
        });
        let started = std::time::Instant::now();
        #[cfg(windows)]
        let long = "Start-Sleep -Seconds 40";
        #[cfg(not(windows))]
        let long = "sleep 40";
        assert_eq!(run_command(long, &stop), Err(COMMAND_STOPPED.to_string()));
        assert!(started.elapsed() < Duration::from_secs(20));
        teller.join().unwrap();
    }

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
