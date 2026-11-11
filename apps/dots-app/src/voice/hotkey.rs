//! The global hotkey that opens Open Dot (Open Dot design spec, step 3).
//!
//! Registered from Rust only, through `tauri-plugin-global-shortcut`, and only while voice is
//! on and a shortcut is set (the service decides when, through `HotkeyPort`). A press shows
//! the main window and opens the session; while Dot speaks or thinks, the session treats that
//! as interrupt-and-listen.

use std::sync::{Mutex, PoisonError};

use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcut, Shortcut, ShortcutState};

use super::service::HotkeyPort;
use super::VoiceHandle;

/// Registration failed: the shortcut is taken.
pub const TAKEN: &str = "That shortcut is already used by another app. Pick a different one.";
/// The setting does not parse as a shortcut.
pub const NOT_A_SHORTCUT: &str = "That is not a shortcut Dots understands.";

/// The shortcut `s` names: `None` for empty (no hotkey), the sentence when it is not one.
/// `CmdOrCtrl` is Command on macOS and Control elsewhere.
pub fn parse_hotkey(s: &str) -> Result<Option<Shortcut>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    s.parse::<Shortcut>()
        .map(Some)
        .map_err(|_| NOT_A_SHORTCUT.to_string())
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

/// The hotkey over the global-shortcut plugin. Each call blocks until the main thread has
/// done it (the plugin's rule), so it is never called from the main thread while that waits.
pub struct PluginHotkey {
    app: AppHandle,
    current: Mutex<Option<Shortcut>>,
}

impl PluginHotkey {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            current: Mutex::new(None),
        }
    }

    fn plugin(&self) -> Option<tauri::State<'_, GlobalShortcut<Wry>>> {
        self.app.try_state::<GlobalShortcut<Wry>>()
    }
}

impl HotkeyPort for PluginHotkey {
    fn register(&self, shortcut: &str) -> Result<(), String> {
        let wanted = parse_hotkey(shortcut)?;
        self.unregister();
        let Some(wanted) = wanted else { return Ok(()) };
        let Some(plugin) = self.plugin() else {
            tracing::warn!("voice: the global shortcut plugin is not there");
            return Err(TAKEN.to_string());
        };
        plugin
            .on_shortcut(wanted, |app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    pressed(app);
                }
            })
            .map_err(|e| {
                tracing::warn!("voice: the hotkey could not be registered: {e}");
                TAKEN.to_string()
            })?;
        *self.current.lock().unwrap_or_else(PoisonError::into_inner) = Some(wanted);
        Ok(())
    }

    fn unregister(&self) {
        let had = self
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let (Some(had), Some(plugin)) = (had, self.plugin()) {
            if let Err(e) = plugin.unregister(had) {
                tracing::warn!("voice: the hotkey could not be unregistered: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_global_shortcut::{Code, Modifiers};

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
}
