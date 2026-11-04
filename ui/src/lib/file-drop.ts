// Files dropped on the window (attachments spec sections 3.1.12 and 5): Tauri hands a drop to
// the window as a whole, with the dropped files' paths, so the whole window is the drop target
// while a chat box can take files.
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect, useRef, useState } from "react";

/// Whether files are being dragged over the window. A drop calls `onPaths` with the dropped
/// paths while `enabled`; otherwise it is ignored.
export function useFileDrop(enabled: boolean, onPaths: (paths: string[]) => void): boolean {
  const [over, setOver] = useState(false);
  // The latest of both, read when an event arrives: the listener is attached once.
  const latest = useRef({ enabled, onPaths });
  latest.current = { enabled, onPaths };
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let gone = false;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const e = event.payload;
        if (e.type === "enter" || e.type === "over") {
          setOver(true);
        } else if (e.type === "leave") {
          setOver(false);
        } else if (e.type === "drop") {
          setOver(false);
          if (latest.current.enabled && e.paths.length > 0) latest.current.onPaths(e.paths);
        }
      })
      .then(
        (stop) => {
          if (gone) stop();
          else unlisten = stop;
        },
        () => {},
      );
    return () => {
      gone = true;
      unlisten?.();
    };
  }, []);
  return over && enabled;
}
