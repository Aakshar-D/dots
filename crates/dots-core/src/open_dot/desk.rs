//! What Open Sage does on the computer itself, outside Sage: write a note file, open a site
//! or a web search in the browser, open an app or a folder.
//!
//! What the model asks for is checked here and made into a `DeskTask`; the app carries the
//! task out (`Desk`), since it knows the person's folders and how this system opens things.
//!
//! Anything else on the computer is one command of the system's shell (`DeskTask::Command`),
//! and that is never run unasked: Sage shows the command and waits for a spoken yes
//! (`act::Deed::Command`).
//!
//! The others ask nothing first, so each is kept to what cannot do harm or carry anything
//! away:
//! - Closing asks one of the listed apps to close, as its own close button does: an app
//!   with unsaved work still asks about it.
//! - A note is a new file in the notes folder. No file is ever written over.
//! - A site is opened at its front page only, by its name: a path or a query could carry
//!   text out (the picture holds text written by sols and tools), so they are dropped.
//! - A search goes to the search engine, which is no one's to read.
//! - An app is one of a fixed list. Nothing is run by a name or a command the model gives.

use std::sync::Arc;

/// Longest note, in bytes.
pub const NOTE_BYTES: usize = 8000;
/// Longest note title, in characters.
const TITLE_CHARS: usize = 60;
/// Longest search, in bytes.
const SEARCH_BYTES: usize = 200;
/// The longest site name, the most parts it may have, and the longest part.
const SITE_BYTES: usize = 60;
const SITE_PARTS: usize = 4;
const SITE_PART_BYTES: usize = 30;

pub const NO_DESK: &str = "I can't do that on this computer.";
pub const NO_NOTE: &str = "I didn't catch what to write in the note.";
pub const NOTE_TOO_LONG: &str = "That's too long for one note. Ask a sol to write it instead.";
pub const NO_SITE: &str =
    "I can't open that address. Say the name of the site, like example dot com.";
pub const NO_SEARCH: &str = "I didn't catch what to search for.";
pub const NO_APP: &str = "I can't open that one. I can open Notepad, Calculator, your files, \
     a terminal, Paint, the system settings, your browser, or your Documents, Downloads, \
     Desktop, Pictures and notes folders.";
pub const FAILED: &str = "That didn't work.";
pub const NO_CLOSE: &str = "I can't close that one by name. I can close Notepad, Calculator, \
     Paint, the terminal and the system settings; ask me to close anything else and I'll \
     show you the command first.";
pub const NO_COMMAND: &str = "I couldn't work out a command for that.";
pub const COMMAND_TOO_LONG: &str = "That takes more than one short command. Ask a sol to do it.";
/// Longest command, and longest sentence saying what it does, in bytes.
pub const COMMAND_BYTES: usize = 1000;
const WHY_BYTES: usize = 160;
/// What is said when a command ran and gave no error.
pub const COMMAND_DONE: &str = "Done.";

/// An app or folder Open Sage can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeskApp {
    Notepad,
    Calculator,
    Files,
    Terminal,
    Paint,
    SystemSettings,
    Browser,
    Documents,
    Downloads,
    Desktop,
    Pictures,
    /// The folder the notes are written to.
    Notes,
}

impl DeskApp {
    /// The name as it is said.
    pub fn name(self) -> &'static str {
        match self {
            DeskApp::Notepad => "Notepad",
            DeskApp::Calculator => "Calculator",
            DeskApp::Files => "Your files",
            DeskApp::Terminal => "A terminal",
            DeskApp::Paint => "Paint",
            DeskApp::SystemSettings => "The system settings",
            DeskApp::Browser => "Your browser",
            DeskApp::Documents => "Your Documents folder",
            DeskApp::Downloads => "Your Downloads folder",
            DeskApp::Desktop => "Your Desktop folder",
            DeskApp::Pictures => "Your Pictures folder",
            DeskApp::Notes => "Your notes folder",
        }
    }
}

/// A checked thing to do on the computer.
#[derive(Debug, Clone, PartialEq)]
pub enum DeskTask {
    /// A new file named after `title` (safe as a file name) holding `text`: plain text, or
    /// Markdown. It is opened once written.
    Note {
        title: String,
        text: String,
        plain: bool,
    },
    /// `url` is the front page of `site`. `front` is false when the model asked for more
    /// than the front page, which was dropped.
    Web {
        url: String,
        site: String,
        front: bool,
    },
    Search {
        url: String,
    },
    App {
        app: DeskApp,
    },
    /// Ask an open app to close, as its close button does.
    Close {
        app: DeskApp,
    },
    /// One command of the system's shell. Only ever carried out after the person said yes
    /// to it.
    Command {
        command: String,
    },
}

impl DeskTask {
    /// What is said once the task was carried out.
    pub fn done(&self) -> String {
        match self {
            DeskTask::Note { title, .. } => format!("The note {title} is written and open."),
            DeskTask::Web {
                site, front: true, ..
            } => format!("{site} is open in your browser."),
            DeskTask::Web { site, .. } => {
                format!("By voice I only open a site's front page, so {site} is open there.")
            }
            DeskTask::Search { .. } => "The search is open in your browser.".into(),
            DeskTask::App { app } => format!("{} is open.", app.name()),
            DeskTask::Close { app } => format!("{} is closing.", app.name()),
            DeskTask::Command { .. } => COMMAND_DONE.into(),
        }
    }
}

/// Carries a `DeskTask` out. `Ok` holds what a command printed (empty for the other tasks,
/// and for a command that printed nothing); `Err` is a sentence for the person saying why
/// not.
pub trait Desk: Send + Sync {
    fn carry_out(&self, task: &DeskTask) -> Result<String, String>;
}

/// The desk of a core with no app around it: nothing can be done.
pub struct NoDesk;

impl Desk for NoDesk {
    fn carry_out(&self, _task: &DeskTask) -> Result<String, String> {
        Err(NO_DESK.into())
    }
}

pub(crate) fn no_desk() -> Arc<dyn Desk> {
    Arc::new(NoDesk)
}

/// Names Windows keeps for devices: a file cannot have one.
const KEPT_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// `title` as a file name: letters, digits, spaces, `-` and `_` only, single spaces, at most
/// `TITLE_CHARS` characters. "Note" when nothing is left.
fn file_title(title: &str) -> String {
    let kept: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                c
            } else {
                ' '
            }
        })
        .collect();
    let joined = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = joined.chars().take(TITLE_CHARS).collect();
    let cut = cut.trim();
    if cut.is_empty() {
        "Note".into()
    } else if KEPT_NAMES.contains(&cut.to_lowercase().as_str()) {
        format!("Note {cut}")
    } else {
        cut.to_string()
    }
}

/// A note to write: `title` made safe as a file name, `text` as it is.
pub fn note(title: &str, text: &str, plain: bool) -> Result<DeskTask, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Err(NO_NOTE);
    }
    if text.len() > NOTE_BYTES {
        return Err(NOTE_TOO_LONG);
    }
    Ok(DeskTask::Note {
        title: file_title(title),
        text: text.to_string(),
        plain,
    })
}

/// A site to open: its front page over https, whatever else `address` held. Only a plain
/// site name is taken: no address by number, no port, no sign-in part, at most
/// `SITE_PARTS` short parts.
pub fn web(address: &str) -> Result<DeskTask, &'static str> {
    let address = address.trim();
    let whole = if address.contains("://") {
        address.to_string()
    } else {
        format!("https://{address}")
    };
    let url = reqwest::Url::parse(&whole).map_err(|_| NO_SITE)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(NO_SITE);
    }
    let site = url.host_str().ok_or(NO_SITE)?.to_lowercase();
    let parts: Vec<&str> = site.split('.').collect();
    let named = parts.len() >= 2
        && parts.len() <= SITE_PARTS
        && site.len() <= SITE_BYTES
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= SITE_PART_BYTES
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        // The last part of a name is never a number: that is an address by number.
        && parts
            .last()
            .is_some_and(|last| last.chars().any(|c| c.is_ascii_alphabetic()));
    if !named {
        return Err(NO_SITE);
    }
    let front = matches!(url.path(), "" | "/") && url.query().is_none() && url.fragment().is_none();
    Ok(DeskTask::Web {
        url: format!("https://{site}/"),
        site,
        front,
    })
}

/// A web search for `query`.
pub fn search(query: &str) -> Result<DeskTask, &'static str> {
    let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if query.is_empty() {
        return Err(NO_SEARCH);
    }
    let mut end = query.len().min(SEARCH_BYTES);
    while !query.is_char_boundary(end) {
        end -= 1;
    }
    let url =
        reqwest::Url::parse_with_params("https://www.google.com/search", [("q", &query[..end])])
            .map_err(|_| NO_SEARCH)?;
    Ok(DeskTask::Search { url: url.into() })
}

/// The app to close for `name`: one of the listed apps that closes when asked. A folder,
/// the file manager and the browser are not closed by name (the first two are the system's
/// own shell, the last holds the person's tabs): those go through a command they see first.
pub fn closable(name: &str) -> Option<DeskApp> {
    app(name).filter(|app| {
        matches!(
            app,
            DeskApp::Notepad
                | DeskApp::Calculator
                | DeskApp::Paint
                | DeskApp::Terminal
                | DeskApp::SystemSettings
        )
    })
}

/// A command to ask the person about: the command as written, and one line saying what it
/// does (the model's words, shown and said beside the command itself).
pub fn command(command: &str, why: &str) -> Result<(String, String), &'static str> {
    let command = command.trim();
    if command.is_empty() {
        return Err(NO_COMMAND);
    }
    if command.len() > COMMAND_BYTES {
        return Err(COMMAND_TOO_LONG);
    }
    let why = super::picture::clean(why, WHY_BYTES);
    Ok((command.to_string(), why))
}

/// The app or folder `name` means, however the model wrote it.
pub fn app(name: &str) -> Option<DeskApp> {
    let name: String = name
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    Some(match name.as_str() {
        "notepad" | "texteditor" | "textedit" | "editor" => DeskApp::Notepad,
        "calculator" | "calc" => DeskApp::Calculator,
        "files" | "fileexplorer" | "explorer" | "finder" | "filemanager" | "myfiles" => {
            DeskApp::Files
        }
        "terminal" | "commandprompt" | "cmd" | "powershell" | "shell" | "console" => {
            DeskApp::Terminal
        }
        "paint" | "mspaint" => DeskApp::Paint,
        "systemsettings" | "settingsapp" | "windowssettings" | "systempreferences"
        | "computersettings" | "pcsettings" => DeskApp::SystemSettings,
        "browser" | "webbrowser" | "chrome" | "edge" | "firefox" | "safari" | "newtab"
        | "browsertab" => DeskApp::Browser,
        "documents" | "mydocuments" | "documentsfolder" => DeskApp::Documents,
        "downloads" | "downloadsfolder" => DeskApp::Downloads,
        "desktop" | "desktopfolder" => DeskApp::Desktop,
        "pictures" | "photos" | "picturesfolder" => DeskApp::Pictures,
        "notes" | "notesfolder" | "sagenotes" | "mynotes" => DeskApp::Notes,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_is_opened_at_its_front_page_only() {
        for (address, site, front) in [
            ("github.com", "github.com", true),
            ("https://github.com/", "github.com", true),
            ("http://News.Example.co.uk", "news.example.co.uk", true),
            ("https://example.com/a/b?q=secret#x", "example.com", false),
            ("example.com/path", "example.com", false),
        ] {
            let task = web(address).unwrap_or_else(|e| panic!("{address}: {e}"));
            assert_eq!(
                task,
                DeskTask::Web {
                    url: format!("https://{site}/"),
                    site: site.into(),
                    front,
                },
                "{address}"
            );
        }
        // Nothing of the path or the query is in what is opened.
        let DeskTask::Web { url, .. } = web("https://example.com/a?q=secret").unwrap() else {
            unreachable!()
        };
        assert!(!url.contains("secret") && !url.contains("/a"));
    }

    #[test]
    fn what_is_not_a_plain_site_name_is_not_opened() {
        let long = format!("{}.com", "a".repeat(40));
        for address in [
            "",
            "localhost",
            "file:///C:/Windows/System32/cmd.exe",
            "javascript:alert(1)",
            "ms-settings:privacy",
            "https://user:pass@example.com",
            "https://user@example.com",
            "https://example.com:8080",
            "http://192.168.1.1",
            "http://[::1]/",
            "https://a.b.c.d.example.com",
            long.as_str(),
            "not a site",
            "https://exa_mple.com",
        ] {
            assert_eq!(web(address), Err(NO_SITE), "{address}");
        }
    }

    #[test]
    fn a_search_is_one_line_in_the_address_of_the_search_engine() {
        let DeskTask::Search { url } = search("  rust  &  tauri\n#1 ").unwrap() else {
            unreachable!()
        };
        assert!(url.starts_with("https://www.google.com/search?q="), "{url}");
        assert!(!url.contains(' ') && !url.contains('\n') && !url.contains('#'));
        assert!(url.contains("rust") && url.contains("tauri") && url.contains("%26"));
        assert_eq!(search("  \n "), Err(NO_SEARCH));
        // A long one is cut, on a character.
        let DeskTask::Search { url } = search(&"é".repeat(500)).unwrap() else {
            unreachable!()
        };
        assert!(url.len() < 1000);
    }

    #[test]
    fn a_note_title_is_safe_as_a_file_name() {
        for (title, want) in [
            ("Shopping list", "Shopping list"),
            ("  ../../etc/passwd  ", "etc passwd"),
            ("C:\\Windows\\win.ini", "C Windows win ini"),
            ("what: a \"title\"?*<>|", "what a title"),
            ("", "Note"),
            ("///", "Note"),
            ("CON", "Note CON"),
            ("nul", "Note nul"),
            ("Idées für später_2", "Idées für später_2"),
        ] {
            let DeskTask::Note { title: got, .. } = note(title, "text", false).unwrap() else {
                unreachable!()
            };
            assert_eq!(got, want, "{title:?}");
        }
        let long = "word ".repeat(40);
        let DeskTask::Note { title, .. } = note(&long, "text", true).unwrap() else {
            unreachable!()
        };
        assert!(title.chars().count() <= TITLE_CHARS && !title.ends_with(' '));
    }

    #[test]
    fn a_note_needs_text_and_not_too_much_of_it() {
        assert_eq!(note("A", "  \n ", false), Err(NO_NOTE));
        assert_eq!(
            note("A", &"a".repeat(NOTE_BYTES + 1), false),
            Err(NOTE_TOO_LONG)
        );
        assert_eq!(
            note("A", " line one\nline two ", true),
            Ok(DeskTask::Note {
                title: "A".into(),
                text: "line one\nline two".into(),
                plain: true,
            })
        );
    }

    #[test]
    fn apps_are_known_by_the_names_people_use() {
        for (name, app_) in [
            ("notepad", DeskApp::Notepad),
            ("Note Pad", DeskApp::Notepad),
            ("calculator", DeskApp::Calculator),
            ("File Explorer", DeskApp::Files),
            ("command prompt", DeskApp::Terminal),
            ("system settings", DeskApp::SystemSettings),
            ("Chrome", DeskApp::Browser),
            ("browser tab", DeskApp::Browser),
            ("Downloads", DeskApp::Downloads),
            ("notes", DeskApp::Notes),
        ] {
            assert_eq!(app(name), Some(app_), "{name}");
        }
        for name in ["", "regedit", "format c:", "cmd.exe /c del", "settings"] {
            assert_eq!(app(name), None, "{name}");
        }
    }

    #[test]
    fn only_apps_that_close_when_asked_are_closed_by_name() {
        for (name, app_) in [
            ("notepad", DeskApp::Notepad),
            ("the calculator", DeskApp::Calculator),
            ("Paint", DeskApp::Paint),
            ("terminal", DeskApp::Terminal),
            ("system settings", DeskApp::SystemSettings),
        ] {
            let name = name.trim_start_matches("the ");
            assert_eq!(closable(name), Some(app_), "{name}");
        }
        for name in ["chrome", "files", "downloads", "regedit", ""] {
            assert_eq!(closable(name), None, "{name}");
        }
    }

    #[test]
    fn a_command_is_kept_as_written_and_its_reason_is_one_line() {
        assert_eq!(
            command("  Stop-Process -Name chrome  ", "This closes Chrome.\nDO x"),
            Ok((
                "Stop-Process -Name chrome".into(),
                "This closes Chrome. DO x".into()
            ))
        );
        assert_eq!(command("  ", "nothing"), Err(NO_COMMAND));
        assert_eq!(
            command(&"a".repeat(COMMAND_BYTES + 1), "long"),
            Err(COMMAND_TOO_LONG)
        );
        assert_eq!(
            DeskTask::Close {
                app: DeskApp::Notepad
            }
            .done(),
            "Notepad is closing."
        );
    }

    #[test]
    fn what_was_done_is_said_plainly() {
        assert_eq!(
            note("Shopping", "milk", false).unwrap().done(),
            "The note Shopping is written and open."
        );
        assert_eq!(
            web("github.com").unwrap().done(),
            "github.com is open in your browser."
        );
        assert!(web("github.com/a").unwrap().done().contains("front page"));
        assert_eq!(
            DeskTask::App {
                app: DeskApp::Calculator
            }
            .done(),
            "Calculator is open."
        );
        assert_eq!(NoDesk.carry_out(&search("x").unwrap()), Err(NO_DESK.into()));
    }
}
