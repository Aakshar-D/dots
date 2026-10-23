import { useEffect, useRef, useState } from "react";

/// How close to the bottom, in pixels, still counts as at the bottom.
const SLACK = 32;

/// Keeps a scrolling element at its bottom while what is in it grows, unless the person has
/// scrolled up; then `behind` is true until they come back down or `jump` takes them there.
/// The element given `ref` must be mounted with the component.
export function useStickToBottom<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const stuck = useRef(true);
  const [behind, setBehind] = useState(false);
  useEffect(() => {
    const element = ref.current;
    if (element === null) return;
    element.scrollTop = element.scrollHeight;
    const observer = new MutationObserver(() => {
      if (stuck.current) element.scrollTop = element.scrollHeight;
      else setBehind(true);
    });
    observer.observe(element, { childList: true, subtree: true, characterData: true });
    return () => observer.disconnect();
  }, []);
  function onScroll() {
    const element = ref.current;
    if (element === null) return;
    stuck.current = element.scrollHeight - element.scrollTop - element.clientHeight <= SLACK;
    if (stuck.current) setBehind(false);
  }
  function jump() {
    const element = ref.current;
    if (element === null) return;
    element.scrollTop = element.scrollHeight;
    stuck.current = true;
    setBehind(false);
  }
  return { ref, onScroll, behind, jump };
}
