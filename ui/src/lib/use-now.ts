import { useEffect, useState } from "react";

/// The current time, refreshed every second while `ticking`, so that the duration of a run
/// that is still going counts up on screen.
export function useNow(ticking: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!ticking) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [ticking]);
  return now;
}
