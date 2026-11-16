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

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Longest note, in bytes.
pub const NOTE_BYTES: usize = 8000;
/// Longest note title, in characters.
const TITLE_CHARS: usize = 60;
/// Longest search, in bytes.
const SEARCH_BYTES: usize = 200;
/// The longest site name, the most parts it may have, and the longest part.
const SITE_BYTES: usize = 50;
const SITE_PARTS: usize = 4;
const SITE_PART_BYTES: usize = 20;

pub const NO_DESK: &str = "I can't do that on this computer.";
pub const NO_NOTE: &str = "I didn't catch what to write in the note.";
pub const NOTE_TOO_LONG: &str = "That's too long for one note. Ask a sol to write it instead.";
pub const NO_SITE: &str =
    "I can't open that address. Say the name of the site, like example dot com.";
pub const NO_SEARCH: &str = "I didn't catch what to search for.";
pub const NO_APP: &str = "I didn't catch which app to open. Say its name, like Chrome or \
     Notepad, or a settings page, like the microphone settings.";
pub const NO_KEY: &str = "I can turn the volume up or down, mute it, play or pause, and skip \
     to the next or the previous track.";
/// The longest name of an app to look for among the installed ones.
const APP_NAME_BYTES: usize = 40;
pub const FAILED: &str = "That didn't work.";
pub const NO_CLOSE: &str = "I can't close that one by name. I can close Notepad, Calculator, \
     Paint and the system settings; ask me to close anything else and I'll show you the \
     command first.";
pub const NO_COMMAND: &str = "I couldn't work out a command for that.";
pub const COMMAND_TOO_LONG: &str = "That takes more than one short command. Ask a sol to do it.";
/// Longest command, and longest sentence saying what it does, in bytes.
pub const COMMAND_BYTES: usize = 300;
pub const COMMAND_NOT_PLAIN: &str =
    "I won't run that one: the command is more than one plain line.";
const WHY_BYTES: usize = 160;
/// What is said when a command ran and gave no error.
pub const COMMAND_DONE: &str = "Done.";
/// Said for a request that needs a command while commands are turned off.
pub const COMMANDS_OFF: &str =
    "That needs a command on your computer, and running commands is turned off in Settings.";
/// Said when a command was ended before it finished.
pub const COMMAND_STOPPED: &str = "I stopped it.";

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

/// A page of the computer's own settings. Each opens by an address of the system that is
/// written here: nothing the model wrote is part of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    Microphone,
    Sound,
    Bluetooth,
    Wifi,
    Network,
    Display,
    Notifications,
    Camera,
    Privacy,
    Updates,
    Apps,
    Power,
    Storage,
    DateTime,
    Keyboard,
    Mouse,
    Background,
    Accounts,
    Language,
    Printers,
}

impl SettingsPage {
    pub const ALL: [SettingsPage; 20] = [
        SettingsPage::Microphone,
        SettingsPage::Sound,
        SettingsPage::Bluetooth,
        SettingsPage::Wifi,
        SettingsPage::Network,
        SettingsPage::Display,
        SettingsPage::Notifications,
        SettingsPage::Camera,
        SettingsPage::Privacy,
        SettingsPage::Updates,
        SettingsPage::Apps,
        SettingsPage::Power,
        SettingsPage::Storage,
        SettingsPage::DateTime,
        SettingsPage::Keyboard,
        SettingsPage::Mouse,
        SettingsPage::Background,
        SettingsPage::Accounts,
        SettingsPage::Language,
        SettingsPage::Printers,
    ];

    /// The name as it is said: "The microphone settings".
    pub fn name(self) -> &'static str {
        match self {
            SettingsPage::Microphone => "The microphone settings",
            SettingsPage::Sound => "The sound settings",
            SettingsPage::Bluetooth => "The Bluetooth settings",
            SettingsPage::Wifi => "The Wi-Fi settings",
            SettingsPage::Network => "The network settings",
            SettingsPage::Display => "The display settings",
            SettingsPage::Notifications => "The notification settings",
            SettingsPage::Camera => "The camera settings",
            SettingsPage::Privacy => "The privacy settings",
            SettingsPage::Updates => "The update settings",
            SettingsPage::Apps => "The settings for installed apps",
            SettingsPage::Power => "The power and battery settings",
            SettingsPage::Storage => "The storage settings",
            SettingsPage::DateTime => "The date and time settings",
            SettingsPage::Keyboard => "The keyboard settings",
            SettingsPage::Mouse => "The mouse settings",
            SettingsPage::Background => "The background settings",
            SettingsPage::Accounts => "The account settings",
            SettingsPage::Language => "The language settings",
            SettingsPage::Printers => "The printer settings",
        }
    }

    /// The page's address in Windows Settings.
    pub fn windows(self) -> &'static str {
        match self {
            SettingsPage::Microphone => "ms-settings:privacy-microphone",
            SettingsPage::Sound => "ms-settings:sound",
            SettingsPage::Bluetooth => "ms-settings:bluetooth",
            SettingsPage::Wifi => "ms-settings:network-wifi",
            SettingsPage::Network => "ms-settings:network-status",
            SettingsPage::Display => "ms-settings:display",
            SettingsPage::Notifications => "ms-settings:notifications",
            SettingsPage::Camera => "ms-settings:privacy-webcam",
            SettingsPage::Privacy => "ms-settings:privacy",
            SettingsPage::Updates => "ms-settings:windowsupdate",
            SettingsPage::Apps => "ms-settings:appsfeatures",
            SettingsPage::Power => "ms-settings:powersleep",
            SettingsPage::Storage => "ms-settings:storagesense",
            SettingsPage::DateTime => "ms-settings:dateandtime",
            SettingsPage::Keyboard => "ms-settings:typing",
            SettingsPage::Mouse => "ms-settings:mousetouchpad",
            SettingsPage::Background => "ms-settings:personalization-background",
            SettingsPage::Accounts => "ms-settings:yourinfo",
            SettingsPage::Language => "ms-settings:regionlanguage",
            SettingsPage::Printers => "ms-settings:printers",
        }
    }

    /// The page's address in macOS System Settings.
    pub fn mac(self) -> &'static str {
        match self {
            SettingsPage::Microphone => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            }
            SettingsPage::Sound => "x-apple.systempreferences:com.apple.preference.sound",
            SettingsPage::Bluetooth => "x-apple.systempreferences:com.apple.preferences.Bluetooth",
            SettingsPage::Wifi | SettingsPage::Network => {
                "x-apple.systempreferences:com.apple.preference.network"
            }
            SettingsPage::Display => "x-apple.systempreferences:com.apple.preference.displays",
            SettingsPage::Notifications => {
                "x-apple.systempreferences:com.apple.preference.notifications"
            }
            SettingsPage::Camera => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera"
            }
            SettingsPage::Privacy => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy"
            }
            SettingsPage::Updates => {
                "x-apple.systempreferences:com.apple.preferences.softwareupdate"
            }
            SettingsPage::Power => "x-apple.systempreferences:com.apple.preference.battery",
            SettingsPage::DateTime => "x-apple.systempreferences:com.apple.preference.datetime",
            SettingsPage::Keyboard => "x-apple.systempreferences:com.apple.preference.keyboard",
            SettingsPage::Mouse => "x-apple.systempreferences:com.apple.preference.mouse",
            SettingsPage::Background => {
                "x-apple.systempreferences:com.apple.preference.desktopscreeneffect"
            }
            SettingsPage::Accounts => "x-apple.systempreferences:com.apple.preferences.users",
            SettingsPage::Language => "x-apple.systempreferences:com.apple.Localization",
            SettingsPage::Printers => "x-apple.systempreferences:com.apple.preference.printfax",
            // No page of their own that is known to open: the settings themselves.
            SettingsPage::Apps | SettingsPage::Storage => "x-apple.systempreferences:",
        }
    }
}

/// A key of the keyboard's volume and media row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeskKey {
    VolumeUp,
    VolumeDown,
    Mute,
    PlayPause,
    Next,
    Previous,
}

impl DeskKey {
    /// What is said once it was pressed.
    pub fn done(self) -> &'static str {
        match self {
            DeskKey::VolumeUp => "The volume is up.",
            DeskKey::VolumeDown => "The volume is down.",
            DeskKey::Mute => "Done.",
            DeskKey::PlayPause => "Done.",
            DeskKey::Next => "Next one.",
            DeskKey::Previous => "Back one.",
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
    /// One page of the computer's own settings.
    Page {
        page: SettingsPage,
    },
    /// An installed app, looked up by `name` among those the system lists: the name is only
    /// compared with theirs, and what is started is the system's own entry for the match.
    Named {
        name: String,
    },
    /// A press of a volume or media key.
    Key {
        key: DeskKey,
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
            DeskTask::Page { page } => format!("{} are open.", page.name()),
            DeskTask::Named { name } => format!("{name} is opening."),
            DeskTask::Key { key } => key.done().into(),
            DeskTask::Close { app } => format!("{} is closing.", app.name()),
            DeskTask::Command { .. } => COMMAND_DONE.into(),
        }
    }
}

/// Carries a `DeskTask` out. `Ok` holds what a command printed (empty for the other tasks,
/// and for a command that printed nothing); `Err` is a sentence for the person saying why
/// not. A command that is running ends soon after `stop` is set (the person said stop, or
/// closed the stage).
pub trait Desk: Send + Sync {
    fn carry_out(&self, task: &DeskTask, stop: &AtomicBool) -> Result<String, String>;
}

/// The desk of a core with no app around it: nothing can be done.
pub struct NoDesk;

impl Desk for NoDesk {
    fn carry_out(&self, _task: &DeskTask, _stop: &AtomicBool) -> Result<String, String> {
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
                // No name written in code for other letters: it can look like another site.
                && !part.starts_with("xn--")
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
/// the file manager, the browser and the terminal are not closed by name (the first two are
/// the system's own shell, the browser holds the person's tabs, and a terminal closes with
/// whatever runs in it and asks nothing): those go through a command they see first.
pub fn closable(name: &str) -> Option<DeskApp> {
    app(name).filter(|app| {
        matches!(
            app,
            DeskApp::Notepad | DeskApp::Calculator | DeskApp::Paint | DeskApp::SystemSettings
        )
    })
}

/// Whether `c` could make a line of text read otherwise than it is: a line break or other
/// control character, or one of the marks that are not drawn or that turn the reading order.
fn hides_text(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202F}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
        )
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
    // The person says yes to what they read. A second line, or a character that hides or
    // reorders text, could put more in the command than the card shows.
    if command.chars().any(hides_text) {
        return Err(COMMAND_NOT_PLAIN);
    }
    let why = super::picture::clean(why, WHY_BYTES);
    Ok((command.to_string(), why))
}

/// `name` in small letters and digits only.
fn bare(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The page of the computer's settings `name` means ("microphone settings", "the Wi-Fi
/// settings page"). `None` when it names no settings, or the settings as a whole.
pub fn settings_page(name: &str) -> Option<SettingsPage> {
    let lower = name.to_lowercase();
    if !(lower.contains("setting") || lower.contains("preference")) {
        return None;
    }
    let words: Vec<String> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    let has = |any: &[&str]| words.iter().any(|w| any.contains(&w.as_str()));
    let joined = bare(&lower);
    Some(if has(&["microphone", "mic", "mike"]) {
        SettingsPage::Microphone
    } else if has(&["camera", "webcam"]) {
        SettingsPage::Camera
    } else if has(&["bluetooth"]) {
        SettingsPage::Bluetooth
    } else if joined.contains("wifi") || has(&["wireless"]) {
        SettingsPage::Wifi
    } else if has(&["network", "internet", "ethernet"]) {
        SettingsPage::Network
    } else if has(&[
        "sound",
        "audio",
        "volume",
        "speaker",
        "speakers",
        "headphones",
    ]) {
        SettingsPage::Sound
    } else if has(&["display", "screen", "brightness", "resolution", "monitor"]) {
        SettingsPage::Display
    } else if has(&["notification", "notifications"]) {
        SettingsPage::Notifications
    } else if has(&["update", "updates"]) {
        SettingsPage::Updates
    } else if has(&["power", "battery", "sleep"]) {
        SettingsPage::Power
    } else if has(&["storage", "disk"]) {
        SettingsPage::Storage
    } else if has(&["date", "time", "clock", "timezone"]) {
        SettingsPage::DateTime
    } else if has(&["keyboard", "typing"]) {
        SettingsPage::Keyboard
    } else if has(&["mouse", "touchpad", "trackpad"]) {
        SettingsPage::Mouse
    } else if has(&[
        "background",
        "wallpaper",
        "personalization",
        "personalisation",
        "theme",
    ]) {
        SettingsPage::Background
    } else if has(&["account", "accounts"]) {
        SettingsPage::Accounts
    } else if has(&["language", "region"]) {
        SettingsPage::Language
    } else if has(&["printer", "printers"]) {
        SettingsPage::Printers
    } else if has(&["privacy"]) {
        SettingsPage::Privacy
    } else if has(&["apps", "programs"]) {
        SettingsPage::Apps
    } else {
        return None;
    })
}

/// The volume or media key `name` means ("volume up", "louder", "mute", "pause", "next").
pub fn key(name: &str) -> Option<DeskKey> {
    let lower = name.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |any: &[&str]| words.iter().any(|w| any.contains(w));
    Some(if has(&["mute", "unmute", "silence"]) {
        DeskKey::Mute
    } else if has(&["up", "louder", "raise", "increase", "higher"]) {
        DeskKey::VolumeUp
    } else if has(&["down", "quieter", "lower", "decrease", "softer"]) {
        DeskKey::VolumeDown
    } else if has(&["next", "skip", "forward"]) {
        DeskKey::Next
    } else if has(&["previous", "back", "last"]) {
        DeskKey::Previous
    } else if has(&["play", "pause", "resume", "playpause", "stop"]) {
        DeskKey::PlayPause
    } else {
        return None;
    })
}

/// The name of an installed app to look for: letters, digits, spaces and the few marks app
/// names carry, single spaces, at most `APP_NAME_BYTES`. `None` when nothing usable is left.
/// It is only ever compared with the names the system lists.
pub fn named(name: &str) -> Option<String> {
    let kept: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '.' | '+' | '-' | '&') {
                c
            } else {
                ' '
            }
        })
        .collect();
    let joined = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    let wanted = joined
        .strip_prefix("the ")
        .or_else(|| joined.strip_prefix("The "))
        .unwrap_or(&joined)
        .trim_end_matches(" app")
        .trim();
    (wanted.chars().any(char::is_alphanumeric) && wanted.len() <= APP_NAME_BYTES)
        .then(|| wanted.to_string())
}

/// Which of the installed apps `wanted` means: `apps` holds each one's name as the system
/// lists it. The same name first, then a name that holds it as a word ("chrome" finds
/// "Google Chrome"), then one that starts with it; of several, the shortest name. Small and capital
/// letters, spaces and marks make no difference.
pub fn pick_app<'a>(wanted: &str, apps: &'a [String]) -> Option<&'a String> {
    let want = bare(wanted);
    if want.len() < 2 {
        return None;
    }
    let shortest = |found: Vec<&'a String>| found.into_iter().min_by_key(|name| name.len());
    let same: Vec<&String> = apps.iter().filter(|a| bare(a) == want).collect();
    if !same.is_empty() {
        return shortest(same);
    }
    let word: Vec<&String> = apps
        .iter()
        .filter(|a| {
            a.to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .any(|w| w == want)
        })
        .collect();
    if !word.is_empty() {
        return shortest(word);
    }
    // Three letters at least: "on" must not find "OneNote".
    let starts: Vec<&String> = apps
        .iter()
        .filter(|a| want.len() >= 3 && bare(a).starts_with(&want))
        .collect();
    if !starts.is_empty() {
        return shortest(starts);
    }
    // Longer names only: "on" must not find "OneNote".
    if want.len() < 4 {
        return None;
    }
    shortest(apps.iter().filter(|a| bare(a).contains(&want)).collect())
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
        "browser" | "webbrowser" | "newtab" | "browsertab" | "mybrowser" => DeskApp::Browser,
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
    fn a_settings_page_is_found_by_its_topic() {
        for (name, page) in [
            ("microphone settings", SettingsPage::Microphone),
            ("the mic settings", SettingsPage::Microphone),
            ("Sound Settings", SettingsPage::Sound),
            ("volume settings", SettingsPage::Sound),
            ("bluetooth settings", SettingsPage::Bluetooth),
            ("wi-fi settings", SettingsPage::Wifi),
            ("wifi settings", SettingsPage::Wifi),
            ("network settings", SettingsPage::Network),
            ("display settings", SettingsPage::Display),
            ("camera settings", SettingsPage::Camera),
            ("windows update settings", SettingsPage::Updates),
            ("battery settings", SettingsPage::Power),
            ("date and time settings", SettingsPage::DateTime),
            ("wallpaper settings", SettingsPage::Background),
            ("printer settings", SettingsPage::Printers),
            ("privacy settings", SettingsPage::Privacy),
            ("system preferences for sound", SettingsPage::Sound),
        ] {
            assert_eq!(settings_page(name), Some(page), "{name}");
        }
        // The settings as a whole, an app, and a topic with no settings named are no page.
        for name in [
            "system settings",
            "settings",
            "camera",
            "microphone",
            "notepad",
            "",
        ] {
            assert_eq!(settings_page(name), None, "{name}");
        }
        // Every page has a name and an address on both systems, made here.
        for page in SettingsPage::ALL {
            assert!(page.name().starts_with("The "), "{page:?}");
            assert!(page.windows().starts_with("ms-settings:"), "{page:?}");
            assert!(
                page.mac().starts_with("x-apple.systempreferences:"),
                "{page:?}"
            );
        }
    }

    #[test]
    fn a_volume_or_media_key_is_found_by_what_was_asked() {
        for (name, want) in [
            ("volume up", DeskKey::VolumeUp),
            ("louder", DeskKey::VolumeUp),
            ("turn it up", DeskKey::VolumeUp),
            ("volume down", DeskKey::VolumeDown),
            ("quieter", DeskKey::VolumeDown),
            ("mute", DeskKey::Mute),
            ("unmute", DeskKey::Mute),
            ("play pause", DeskKey::PlayPause),
            ("pause", DeskKey::PlayPause),
            ("next", DeskKey::Next),
            ("skip", DeskKey::Next),
            ("previous", DeskKey::Previous),
        ] {
            assert_eq!(key(name), Some(want), "{name}");
        }
        for name in ["", "enter", "delete everything", "f5"] {
            assert_eq!(key(name), None, "{name}");
        }
    }

    #[test]
    fn an_app_name_is_only_ever_a_plain_name() {
        assert_eq!(named("Chrome").as_deref(), Some("Chrome"));
        assert_eq!(
            named("  the  Task   Manager app ").as_deref(),
            Some("Task Manager")
        );
        assert_eq!(named("Notepad++").as_deref(), Some("Notepad++"));
        // What is no part of a name is dropped, never passed on.
        assert_eq!(named("calc; rm -rf /").as_deref(), Some("calc rm -rf"));
        assert_eq!(named("a\"b`c$(d)").as_deref(), Some("a b c d"));
        for name in ["", "   ", "\"\"", &"a".repeat(41)] {
            assert_eq!(named(name), None, "{name}");
        }
    }

    #[test]
    fn the_installed_app_meant_is_picked_by_name() {
        let apps: Vec<String> = [
            "Google Chrome",
            "Chrome Remote Desktop",
            "Word",
            "WordPad",
            "Task Manager",
            "Visual Studio Code",
            "OneNote",
            "Outlook",
            "Outlook (classic)",
        ]
        .map(String::from)
        .to_vec();
        for (wanted, found) in [
            ("chrome", "Google Chrome"),
            ("google chrome", "Google Chrome"),
            ("word", "Word"),
            ("WORDPAD", "WordPad"),
            ("task manager", "Task Manager"),
            ("taskmanager", "Task Manager"),
            ("visual studio", "Visual Studio Code"),
            ("code", "Visual Studio Code"),
            ("outlook", "Outlook"),
        ] {
            assert_eq!(
                pick_app(wanted, &apps).map(String::as_str),
                Some(found),
                "{wanted}"
            );
        }
        for wanted in ["", "x", "on", "spotify", "ord"] {
            assert_eq!(pick_app(wanted, &apps), None, "{wanted}");
        }
    }

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
            "https://xn--pple-43d.com",
            "https://tok-abcdefghijklmnopqrstuv.example.com",
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
            ("my browser", DeskApp::Browser),
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
            ("system settings", DeskApp::SystemSettings),
        ] {
            let name = name.trim_start_matches("the ");
            assert_eq!(closable(name), Some(app_), "{name}");
        }
        for name in ["chrome", "files", "downloads", "regedit", "terminal", ""] {
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
        // What the card would not show as it is, is not run: a second line far below the
        // first, a mark that turns the reading order, a character that is not drawn.
        for hidden in [
            "Get-Date\n\n\n\n; Remove-Item x",
            "Get-Date\r; Remove-Item x",
            "Get-Date \u{202E}x metI-evomeR",
            "Get-\u{200B}Date",
            "Get-Date\t; x",
        ] {
            assert_eq!(
                command(hidden, "shows the date"),
                Err(COMMAND_NOT_PLAIN),
                "{hidden:?}"
            );
        }
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
        assert_eq!(
            NoDesk.carry_out(&search("x").unwrap(), &AtomicBool::new(false)),
            Err(NO_DESK.into())
        );
    }
}
