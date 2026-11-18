import { useEffect, useRef, useState } from "react";

/// How close to the bottom, in pixels, still counts as at the bottom.
const SLACK = 32;

/// Whether a scroll from `last` to `top` was the person moving up, away from the bottom, and so
/// the end of following it; `distance` is how far `top` is from the bottom. Following only ever
/// moves down. A view that got shorter also moves the top up, but leaves it at the bottom.
export function movedUp(last: number, top: number, distance: number): boolean {
  return top < last && distance > 1;
}

/// Keeps a scrolling element at its bottom while what is in it grows, unless the person has
/// scrolled up; then `behind` is true until they come back down or `jump` takes them there.
/// The element given `ref` must be mounted with the component.
///
/// Any move up lets go, however small. A working sol changes its line several times a second,
/// and each change takes a following view back to the bottom: were the first few pixels of a
/// scroll up still counted as "at the bottom", a slow scroll (a touchpad, a smooth wheel) would
/// be taken back before it got anywhere, and the person could not scroll up while a sol works.
export function useStickToBottom<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const stuck = useRef(true);
  /// Where the top was at the last scroll, or where following last put it.
  const last = useRef(0);
  const [behind, setBehind] = useState(false);
  useEffect(() => {
    const element = ref.current;
    if (element === null) return;
    const follow = () => {
      element.scrollTop = element.scrollHeight;
      last.current = element.scrollTop;
    };
    follow();
    const observer = new MutationObserver(() => {
      if (stuck.current) follow();
      else setBehind(true);
    });
    observer.observe(element, { childList: true, subtree: true, characterData: true });
    return () => observer.disconnect();
  }, []);
  function onScroll() {
    const element = ref.current;
    if (element === null) return;
    const top = element.scrollTop;
    const distance = element.scrollHeight - top - element.clientHeight;
    if (movedUp(last.current, top, distance)) stuck.current = false;
    else if (distance <= SLACK) stuck.current = true;
    last.current = top;
    if (stuck.current) setBehind(false);
  }
  function jump() {
    const element = ref.current;
    if (element === null) return;
    element.scrollTop = element.scrollHeight;
    last.current = element.scrollTop;
    stuck.current = true;
    setBehind(false);
  }
  return { ref, onScroll, behind, jump };
}
