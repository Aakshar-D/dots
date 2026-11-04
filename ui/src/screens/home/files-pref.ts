/// Where the home keeps whether the Files panel is hidden (attachments spec section 6): one
/// choice for every dot, on this device.
export const FILES_KEY = "dots.home.files";

/// Whether the Files panel is hidden. Storage that cannot be read counts as shown.
export function filesHidden(): boolean {
  try {
    return window.localStorage.getItem(FILES_KEY) === "hidden";
  } catch {
    return false;
  }
}

/// Remembers the choice: hidden is kept, shown is the absence of the key. Storage that cannot
/// be written is left alone.
export function setFilesHidden(hidden: boolean): void {
  try {
    if (hidden) window.localStorage.setItem(FILES_KEY, "hidden");
    else window.localStorage.removeItem(FILES_KEY);
  } catch {
    // Shown next time is fine.
  }
}
