/** The current time, refreshed every `everyMs` while there is a start time to count from. */

import { useEffect, useState } from "react";

export function useNow(sinceMs: number | null, everyMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (sinceMs === null) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), everyMs);
    return () => clearInterval(t);
  }, [sinceMs, everyMs]);
  return now;
}
