/// Where the home keeps the dot last opened, so that it opens on it next time.
const KEY = "dots.home.dot";

/// The dot last opened, or null. Storage that cannot be read counts as empty.
export function lastDot(): string | null {
  try {
    return window.localStorage.getItem(KEY);
  } catch {
    return null;
  }
}

/// Remembers `dotId` as the dot last opened; storage that cannot be written is left alone.
export function rememberDot(dotId: string): void {
  try {
    window.localStorage.setItem(KEY, dotId);
  } catch {
    // Opening the first dot next time is fine.
  }
}
