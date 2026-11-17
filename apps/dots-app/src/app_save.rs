//! An app's screen saves a file (`dots.saveFile`, app library spec section 5). The page runs in
//! a frame sandboxed with `allow-scripts` only, so WebView2 refuses any download it starts
//! itself. It hands Dots the file instead, and the person picks where it goes in the Save
//! dialog; the page learns only whether it was saved, never where.

use base64::Engine as _;
use tauri::AppHandle;

use crate::error::CmdError;

/// The largest file an app's screen may save.
pub const MAX_BYTES: usize = 20 * 1024 * 1024;
/// The longest name the Save dialog suggests, in characters.
const MAX_NAME: usize = 200;

/// The name the Save dialog suggests for `name`: its last part (a page names no folder), with
/// each run of characters a Windows file name cannot hold made one `-`, without leading or
/// trailing spaces and dots. An error when nothing is left.
pub fn file_name(name: &str) -> Result<String, String> {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut out = String::with_capacity(last.len());
    let mut bad = false;
    for c in last.chars() {
        if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
            if !bad {
                out.push('-');
            }
            bad = true;
        } else {
            out.push(c);
            bad = false;
        }
    }
    let out: String = out
        .trim_matches([' ', '.'])
        .chars()
        .take(MAX_NAME)
        .collect::<String>()
        .trim_end_matches([' ', '.'])
        .to_owned();
    if out.is_empty() || out == "-" {
        return Err(format!("{name:?} is not a file name"));
    }
    Ok(out)
}

/// The file's bytes from its base64, at most `MAX_BYTES` of them.
pub fn decode(data: &str) -> Result<Vec<u8>, String> {
    // Four characters hold three bytes: anything longer is too large before it is decoded.
    if data.len() > MAX_BYTES.div_ceil(3) * 4 {
        return Err(too_large());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|e| format!("the file is not base64: {e}"))?;
    if bytes.len() > MAX_BYTES {
        return Err(too_large());
    }
    Ok(bytes)
}

fn too_large() -> String {
    format!("the file is larger than {} MB", MAX_BYTES / (1024 * 1024))
}

/// The extension the Save dialog filters on: `pdf` for `INV-1.pdf`, none for a name without one.
pub fn extension(name: &str) -> Option<&str> {
    let (stem, ext) = name.rsplit_once('.')?;
    let fits = !stem.is_empty()
        && (1..=10).contains(&ext.len())
        && ext.chars().all(|c| c.is_ascii_alphanumeric());
    fits.then_some(ext)
}

/// Asks the person where to save `data` (base64) as `name`, and writes it there. False when
/// they cancel. Checked before the dialog opens, so a bad file never asks.
pub fn save(app: &AppHandle, name: &str, data: &str) -> Result<bool, CmdError> {
    use tauri_plugin_dialog::DialogExt;
    let name = file_name(name).map_err(CmdError::invalid)?;
    let bytes = decode(data).map_err(CmdError::invalid)?;
    let mut dialog = app.dialog().file().set_file_name(&name);
    if let Some(ext) = extension(&name) {
        dialog = dialog.add_filter(ext.to_ascii_uppercase(), &[ext]);
    }
    let Some(path) = dialog.blocking_save_file() else {
        return Ok(false);
    };
    let path = path
        .into_path()
        .map_err(|e| CmdError::internal(format!("that place cannot be written: {e}")))?;
    std::fs::write(&path, bytes)
        .map_err(|e| CmdError::internal(format!("could not write {}: {e}", path.display())))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_name_keeps_what_windows_can_hold() {
        assert_eq!(
            file_name("INV-0001 - Acme Ltd.pdf").unwrap(),
            "INV-0001 - Acme Ltd.pdf"
        );
        assert_eq!(file_name("Café résumé.pdf").unwrap(), "Café résumé.pdf");
    }

    #[test]
    fn a_file_name_names_no_folder() {
        assert_eq!(file_name(r"..\..\Windows\evil.exe").unwrap(), "evil.exe");
        assert_eq!(file_name("../a/b/report.csv").unwrap(), "report.csv");
        assert_eq!(file_name(r"C:\Users\u\x.pdf").unwrap(), "x.pdf");
    }

    #[test]
    fn a_file_name_makes_each_run_of_bad_characters_one_dash() {
        assert_eq!(file_name("a:b?*c\u{0}\u{1f}.pdf").unwrap(), "a-b-c-.pdf");
        assert_eq!(file_name("  .hidden.pdf. . ").unwrap(), "hidden.pdf");
    }

    #[test]
    fn a_file_name_with_nothing_left_is_refused() {
        for name in ["", " ", "..", "a/", "a\\..", "???", "con/ . "] {
            assert!(file_name(name).is_err(), "{name:?}");
        }
    }

    #[test]
    fn a_long_file_name_is_cut_without_a_trailing_dot() {
        let long = format!("{}.{}", "a".repeat(199), "pdf");
        let cut = file_name(&long).unwrap();
        assert_eq!(cut.chars().count(), 199, "the 200th character was the dot");
        assert!(!cut.ends_with('.'));
    }

    #[test]
    fn the_data_is_decoded_from_base64() {
        assert_eq!(decode("JVBERi0xLjQK").unwrap(), b"%PDF-1.4\n");
        assert_eq!(decode("").unwrap(), b"");
        assert!(decode("not base64!").is_err());
    }

    #[test]
    fn data_larger_than_the_limit_is_refused() {
        let at_limit = base64::engine::general_purpose::STANDARD.encode(vec![0u8; MAX_BYTES]);
        assert_eq!(decode(&at_limit).unwrap().len(), MAX_BYTES);
        let over = base64::engine::general_purpose::STANDARD.encode(vec![0u8; MAX_BYTES + 1]);
        assert_eq!(decode(&over).unwrap_err(), "the file is larger than 20 MB");
    }

    #[test]
    fn the_dialog_filters_on_a_plain_extension_only() {
        assert_eq!(extension("INV-1.pdf"), Some("pdf"));
        assert_eq!(extension("data.tar.gz"), Some("gz"));
        assert_eq!(extension("README"), None);
        assert_eq!(extension(".pdf"), None);
        assert_eq!(extension("a.p d f"), None);
    }
}
