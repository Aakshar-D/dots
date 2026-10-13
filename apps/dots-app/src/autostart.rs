use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;

/// What to do with the login entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Register,
    Remove,
    Leave,
}

/// A debug build leaves the entry alone (it would point at a development binary), and so does
/// an instance whose data folder comes from `DOTS_DATA_DIR` (the entry belongs to the
/// installed Dots and would start a smoke binary on the real data at the next login).
pub fn entry(debug_build: bool, data_dir_overridden: bool, enabled: bool) -> Entry {
    match (debug_build || data_dir_overridden, enabled) {
        (true, _) => Entry::Leave,
        (false, true) => Entry::Register,
        (false, false) => Entry::Remove,
    }
}

/// Registers or removes the login entry (which starts Dots with `--hidden`), unless `entry`
/// says to leave it.
pub fn apply(app: &AppHandle, enabled: bool, data_dir_overridden: bool) {
    let launcher = app.autolaunch();
    let result = match entry(cfg!(debug_assertions), data_dir_overridden, enabled) {
        Entry::Leave if data_dir_overridden => {
            tracing::info!("DOTS_DATA_DIR is set: autostart left unchanged (wanted {enabled})");
            return;
        }
        Entry::Leave => {
            tracing::info!("debug build: autostart left unchanged (wanted {enabled})");
            return;
        }
        Entry::Register => launcher.enable(),
        Entry::Remove => launcher.disable(),
    };
    if let Err(e) = result {
        tracing::warn!("updating autostart failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_release_build_with_its_own_login_entry_changes_it() {
        use Entry::*;
        assert_eq!(entry(false, false, true), Register);
        assert_eq!(entry(false, false, false), Remove);
        assert_eq!(entry(true, false, true), Leave);
        assert_eq!(entry(true, false, false), Leave);
        // `DOTS_DATA_DIR`: the entry belongs to the installed Dots.
        assert_eq!(entry(false, true, true), Leave);
        assert_eq!(entry(false, true, false), Leave);
        assert_eq!(entry(true, true, true), Leave);
    }
}
