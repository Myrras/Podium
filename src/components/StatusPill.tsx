/**
 * One status pill: a process's state word with its glyph, the time in that
 * state and, while working, the sub-agents and shells running behind it.
 * The count form (`count`) is glyph plus number, for roll-ups.
 */

import { useEffect, useState } from "react";

import type { ProcessKind } from "../ipc/types";
import { formatSince, pillWord, type PillState } from "../lib/processStatus";
import styles from "./StatusPill.module.css";

const TONE: Record<PillState, "work" | "needs" | "crash" | "plain"> = {
  working: "work",
  needs: "needs",
  crashed: "crash",
  idle: "plain",
  up: "plain",
  stopping: "plain",
  exited: "plain",
  notStarted: "plain",
};

const SVG = {
  fill: "none",
  stroke: "currentColor",
  strokeLinecap: "round",
  strokeLinejoin: "round",
  "aria-hidden": true,
} as const;

function Glyph({ state }: { state: PillState }) {
  switch (state) {
    case "working":
      return (
        <span className={styles.bars} aria-hidden="true">
          <i />
          <i />
          <i />
        </span>
      );
    case "needs":
      return <span className={styles.needsDot} aria-hidden="true" />;
    case "idle":
      return <span className={styles.ring} aria-hidden="true" />;
    case "up":
      return <span className={styles.upDot} aria-hidden="true" />;
    case "crashed":
      return (
        <svg viewBox="0 0 8 8" width="8" height="8" {...SVG}>
          <path d="M1.5 1.5l5 5M6.5 1.5l-5 5" strokeWidth="1.6" />
        </svg>
      );
    default:
      return null;
  }
}

function Counter({
  n,
  noun,
  children,
}: {
  n: number;
  noun: string;
  children: React.ReactNode;
}) {
  const label = `${n} ${noun}${n === 1 ? "" : "s"} running`;
  return (
    <span className={styles.bg} role="img" aria-label={label} title={label}>
      {children}
      {n}
    </span>
  );
}

export function StatusPill({
  state,
  kind = "agent",
  sinceMs = null,
  subagents = 0,
  shells = 0,
  count,
}: {
  state: PillState;
  kind?: ProcessKind["kind"];
  /** Epoch ms the state began; `null` shows no time. */
  sinceMs?: number | null;
  subagents?: number;
  shells?: number;
  /** Count form: glyph plus this number instead of the word. */
  count?: number;
}) {
  // One minute is the finest unit shown, so a slow tick is enough.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (sinceMs === null) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 15_000);
    return () => clearInterval(timer);
  }, [sinceMs]);

  const tone = TONE[state];
  const className = `${styles.pill} ${styles[tone]}`;

  if (count !== undefined) {
    const label = `${count} ${pillWord(state).toLowerCase()}`;
    return (
      <span
        className={className}
        data-tone={tone}
        role="img"
        aria-label={label}
        title={label}
      >
        <Glyph state={state} />
        {count}
      </span>
    );
  }

  const since = formatSince(sinceMs, now);
  const working = state === "working";
  return (
    <span className={className} data-tone={tone}>
      <Glyph state={state} />
      {pillWord(state, kind)}
      {since && (
        <span className={styles.time} data-pill-time="">
          {since}
        </span>
      )}
      {working && (subagents > 0 || shells > 0) && (
        <span className={styles.bgGroup}>
          {subagents > 0 && (
            <Counter n={subagents} noun="sub-agent">
              <svg viewBox="0 0 12 12" {...SVG}>
                <path
                  d="M1.5 6h3M4.5 6c2.6 0 2.2-3 5-3M4.5 6c2.6 0 2.2 3 5 3"
                  strokeWidth="1.4"
                />
              </svg>
            </Counter>
          )}
          {shells > 0 && (
            <Counter n={shells} noun="background shell">
              <svg viewBox="0 0 12 12" {...SVG}>
                <path d="M2.5 3.5L5 6 2.5 8.5M6.5 9h3" strokeWidth="1.4" />
              </svg>
            </Counter>
          )}
        </span>
      )}
    </span>
  );
}
