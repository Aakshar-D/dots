import { useEffect } from "react";

import { commands } from "@/api/commands";

/// The calls go to the core one after another, so that the last word on what is viewed
/// arrives last: a switch says "none" for the dot left, then names the dot opened.
let queue: Promise<unknown> = Promise.resolve();

/// Queues `call`. A failure shows nothing: the next change sends again (needs-you spec 5).
function send(call: () => Promise<unknown>): void {
  queue = queue.then(call).catch(() => undefined);
}

/// While the conversation of `dotId` is open (an older topic of it counts) and the window is in
/// front, visible and focused, the core is told that the person is viewing the dot and has seen
/// it; when the window goes behind or the conversation closes, that nothing is viewed
/// (needs-you spec section 5). Each change sends again.
export function useViewing(dotId: string): void {
  useEffect(() => {
    let focused = document.hasFocus();
    const update = () => {
      const inFront = focused && document.visibilityState === "visible";
      send(() => commands.setViewing(inFront ? dotId : null));
      if (inFront) send(() => commands.markDotSeen(dotId));
    };
    const onFocus = () => {
      focused = true;
      update();
    };
    const onBlur = () => {
      focused = false;
      update();
    };
    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);
    document.addEventListener("visibilitychange", update);
    update();
    return () => {
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
      document.removeEventListener("visibilitychange", update);
      send(() => commands.setViewing(null));
    };
  }, [dotId]);
}
