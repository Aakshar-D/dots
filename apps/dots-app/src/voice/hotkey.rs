//! The global hotkey that opens Open Dot (Open Dot design spec, step 3).
//!
//! Registered from Rust only, through `tauri-plugin-global-shortcut`, and only while voice is
//! on and a shortcut is set (the service decides when, through `HotkeyPort`). A press shows
//! the main window and opens the session; while Dot speaks or thinks, the session treats that
//! as interrupt-and-listen.

use std::sync::{Mutex, PoisonError};

use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{Code, GlobalShortcut, Shortcut, ShortcutState};

use super::service::HotkeyPort;
use super::VoiceHandle;

/// Registration failed: the shortcut is taken.
pub const TAKEN: &str = "That shortcut is already used by another app. Pick a different one.";
/// Registration failed for another reason.
pub const COULD_NOT: &str = "Sage could not set that shortcut.";
/// The setting does not parse as a shortcut.
pub const NOT_A_SHORTCUT: &str = "That is not a shortcut Sage understands.";
/// A plain key with no modifier would take that key from every app.
pub const NO_MODIFIER: &str = "Add Ctrl, Alt or Shift to that shortcut, or use a function key.";

/// F1 to F24: the keys that may be a shortcut on their own.
fn function_key(code: Code) -> bool {
    use Code::*;
    matches!(
        code,
        F1 | F2
            | F3
            | F4
            | F5
            | F6
            | F7
            | F8
            | F9
            | F10
            | F11
            | F12
            | F13
            | F14
            | F15
            | F16
            | F17
            | F18
            | F19
            | F20
            | F21
            | F22
            | F23
            | F24
    )
}

/// The shortcut `s` names: `None` for empty (no hotkey), the sentence when it is not one.
/// `CmdOrCtrl` is Command on macOS and Control elsewhere. A key with no modifier is refused
/// unless it is F1 to F24. This parse is the authority on shortcuts: the settings check in
/// the core only checks the shape.
pub fn parse_hotkey(s: &str) -> Result<Option<Shortcut>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let shortcut = s
        .parse::<Shortcut>()
        .map_err(|_| NOT_A_SHORTCUT.to_string())?;
    if shortcut.mods.is_empty() && !function_key(shortcut.key) {
        return Err(NO_MODIFIER.to_string());
    }
    Ok(Some(shortcut))
}

/// The sentence for a plugin error on registering: "taken" only when it says so.
fn register_sentence(error: &str) -> &'static str {
    let e = error.to_ascii_lowercase();
    if e.contains("already registered") || e.contains("in use") {
        TAKEN
    } else {
        COULD_NOT
    }
}

/// A press (not a release; Windows registers with no auto-repeat): the window comes forward,
/// then Open Dot listens.
fn pressed(app: &AppHandle) {
    crate::show_main(app);
    if let Some(voice) = app.try_state::<VoiceHandle>() {
        if let Ok(service) = voice.get() {
            service.open();
        }
    }
}

/// What the plugin does with one shortcut; a fake in tests.
trait Shortcuts {
    fn register(&self, shortcut: Shortcut) -> Result<(), String>;
    fn unregister(&self, shortcut: Shortcut) -> Result<(), String>;
}

/// The shortcuts Dots holds: the one in use, and any that could not be let go, which are
/// tried again when voice is turned off (or taken back when picked again).
#[derive(Default)]
struct Held {
    current: Option<Shortcut>,
    stuck: Vec<Shortcut>,
}

impl Held {
    /// Registers `wanted` in place of the one before, which is let go only once it works.
    fn register(&mut self, plugin: &dyn Shortcuts, wanted: Shortcut) -> Result<(), String> {
        if self.current == Some(wanted) {
            return Ok(());
        }
        if let Some(i) = self.stuck.iter().position(|s| *s == wanted) {
            // Never let go: it still works as it is.
            self.stuck.remove(i);
        } else {
            plugin.register(wanted).map_err(|e| {
                tracing::warn!("voice: the hotkey could not be registered: {e}");
                register_sentence(&e).to_string()
            })?;
        }
        if let Some(old) = self.current.replace(wanted) {
            self.let_go(plugin, old);
        }
        Ok(())
    }

    /// Lets go of every shortcut held; one that cannot be let go stays held.
    fn unregister(&mut self, plugin: &dyn Shortcuts) {
        let all: Vec<Shortcut> = self
            .current
            .take()
            .into_iter()
            .chain(self.stuck.drain(..))
            .collect();
        for s in all {
            self.let_go(plugin, s);
        }
    }

    fn let_go(&mut self, plugin: &dyn Shortcuts, shortcut: Shortcut) {
        if let Err(e) = plugin.unregister(shortcut) {
            tracing::warn!("voice: a hotkey could not be unregistered: {e}");
            self.stuck.push(shortcut);
        }
    }
}

impl Shortcuts for GlobalShortcut<Wry> {
    fn register(&self, shortcut: Shortcut) -> Result<(), String> {
        self.on_shortcut(shortcut, |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                pressed(app);
            }
        })
        .map_err(|e| e.to_string())
    }
    fn unregister(&self, shortcut: Shortcut) -> Result<(), String> {
        GlobalShortcut::unregister(self, shortcut).map_err(|e| e.to_string())
    }
}

/// The hotkey over the global-shortcut plugin. Each call blocks until the main thread has
/// done it (the plugin's rule), so it is never called from the main thread while that waits.
pub struct PluginHotkey {
    app: AppHandle,
    held: Mutex<Held>,
}

impl PluginHotkey {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            held: Mutex::new(Held::default()),
        }
    }

    fn plugin(&self) -> Option<tauri::State<'_, GlobalShortcut<Wry>>> {
        self.app.try_state::<GlobalShortcut<Wry>>()
    }
}

impl HotkeyPort for PluginHotkey {
    /// The new shortcut is registered first; the old one is let go only once it works, so a
    /// shortcut another app holds leaves the old one working.
    fn register(&self, shortcut: &str) -> Result<(), String> {
        let Some(wanted) = parse_hotkey(shortcut)? else {
            self.unregister();
            return Ok(());
        };
        let Some(plugin) = self.plugin() else {
            tracing::warn!("voice: the global shortcut plugin is not there");
            return Err(COULD_NOT.to_string());
        };
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        held.register(&*plugin, wanted)
    }

    fn unregister(&self) {
        let Some(plugin) = self.plugin() else { return };
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        held.unregister(&*plugin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_global_shortcut::Modifiers;

    #[test]
    fn the_default_parses() {
        let default = dots_core::settings::Settings::default().voice_hotkey;
        let shortcut = parse_hotkey(&default).unwrap().unwrap();
        assert_eq!(shortcut.key, Code::Space);
        assert!(shortcut.mods.contains(Modifiers::SHIFT));
    }

    #[test]
    fn cmd_or_ctrl_is_command_on_a_mac_and_control_elsewhere() {
        let shortcut = parse_hotkey("CmdOrCtrl+Shift+Space").unwrap().unwrap();
        let expected = if cfg!(target_os = "macos") {
            Modifiers::SUPER | Modifiers::SHIFT
        } else {
            Modifiers::CONTROL | Modifiers::SHIFT
        };
        assert_eq!(shortcut.mods, expected);
    }

    /// What a string parses to: none, the modifiers and key, or the sentence.
    type Parsed = Result<Option<(Modifiers, Code)>, &'static str>;

    #[test]
    fn parse_table() {
        let cases: &[(&str, Parsed)] = &[
            ("", Ok(None)),
            ("   ", Ok(None)),
            ("Alt+F9", Ok(Some((Modifiers::ALT, Code::F9)))),
            ("F9", Ok(Some((Modifiers::empty(), Code::F9)))),
            ("F24", Ok(Some((Modifiers::empty(), Code::F24)))),
            ("K", Err(NO_MODIFIER)),
            ("Space", Err(NO_MODIFIER)),
            ("Escape", Err(NO_MODIFIER)),
            ("Shift+K", Ok(Some((Modifiers::SHIFT, Code::KeyK)))),
            (
                "Ctrl+Alt+K",
                Ok(Some((Modifiers::CONTROL | Modifiers::ALT, Code::KeyK))),
            ),
            ("Ctrl+", Err(NOT_A_SHORTCUT)),
            ("Ctrl+Shift", Err(NOT_A_SHORTCUT)),
            ("Ctrl+C+Shift", Err(NOT_A_SHORTCUT)),
            ("banana", Err(NOT_A_SHORTCUT)),
            ("Ctrl+Space Bar", Err(NOT_A_SHORTCUT)),
        ];
        for (input, want) in cases {
            let got = parse_hotkey(input).map(|s| s.map(|s| (s.mods, s.key)));
            let want = want.map_err(|e| e.to_string());
            assert_eq!(got, want, "{input:?}");
        }
    }

    /// The plugin, faked: refuses to register what `refuse` names with `error`, and to let go
    /// of what `stuck` names.
    #[derive(Default)]
    struct FakePlugin {
        calls: Mutex<Vec<String>>,
        refuse: Mutex<Option<(Shortcut, String)>>,
        stuck: Mutex<Vec<Shortcut>>,
    }

    impl Shortcuts for FakePlugin {
        fn register(&self, s: Shortcut) -> Result<(), String> {
            self.calls.lock().unwrap().push(format!("register {s}"));
            match &*self.refuse.lock().unwrap() {
                Some((r, e)) if *r == s => Err(e.clone()),
                _ => Ok(()),
            }
        }
        fn unregister(&self, s: Shortcut) -> Result<(), String> {
            self.calls.lock().unwrap().push(format!("unregister {s}"));
            if self.stuck.lock().unwrap().contains(&s) {
                return Err("Failed to unregister hotkey".into());
            }
            Ok(())
        }
    }

    fn key(s: &str) -> Shortcut {
        parse_hotkey(s).unwrap().unwrap()
    }

    #[test]
    fn a_shortcut_that_could_not_be_let_go_is_let_go_when_voice_turns_off() {
        let plugin = FakePlugin::default();
        let mut held = Held::default();
        held.register(&plugin, key("Alt+F9")).unwrap();
        plugin.stuck.lock().unwrap().push(key("Alt+F9"));
        held.register(&plugin, key("Alt+F10")).unwrap();
        assert_eq!(held.stuck, vec![key("Alt+F9")], "still tracked");
        // Free again: turning voice off lets go of both.
        plugin.stuck.lock().unwrap().clear();
        held.unregister(&plugin);
        let calls = plugin.calls.lock().unwrap().clone();
        assert!(
            calls.ends_with(&[
                format!("unregister {}", key("Alt+F10")),
                format!("unregister {}", key("Alt+F9")),
            ]),
            "{calls:?}"
        );
        assert!(held.current.is_none() && held.stuck.is_empty());
    }

    #[test]
    fn a_stuck_shortcut_picked_again_is_taken_back_without_registering() {
        let plugin = FakePlugin::default();
        let mut held = Held::default();
        held.register(&plugin, key("Alt+F9")).unwrap();
        plugin.stuck.lock().unwrap().push(key("Alt+F9"));
        held.register(&plugin, key("Alt+F10")).unwrap();
        plugin.stuck.lock().unwrap().clear();
        held.register(&plugin, key("Alt+F9")).unwrap();
        assert_eq!(held.current, Some(key("Alt+F9")));
        assert!(held.stuck.is_empty());
        let registers = plugin
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.starts_with("register"))
            .count();
        assert_eq!(registers, 2, "Alt+F9 was never registered twice");
    }

    #[test]
    fn only_a_shortcut_in_use_is_called_taken() {
        let cases = [
            ("HotKey already registered: HotKey { .. }", TAKEN),
            ("the shortcut is in use", TAKEN),
            (
                "Unable to register hotkey: Unknown VKCode for F24",
                COULD_NOT,
            ),
            ("Access is denied. (os error 5)", COULD_NOT),
        ];
        for (error, sentence) in cases {
            let plugin = FakePlugin::default();
            *plugin.refuse.lock().unwrap() = Some((key("Alt+F9"), error.to_string()));
            let mut held = Held::default();
            held.register(&plugin, key("Alt+F8")).unwrap();
            assert_eq!(
                held.register(&plugin, key("Alt+F9")),
                Err(sentence.to_string()),
                "{error}"
            );
            // The one before still works.
            assert_eq!(held.current, Some(key("Alt+F8")));
        }
    }
}
