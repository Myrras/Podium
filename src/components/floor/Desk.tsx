import { type ReactElement, useEffect, useState } from "react";
import type { ProcessInfo } from "../../ipc/types";
import { deskState } from "../../lib/floor/rooms";
import { formatSince } from "../../lib/processStatus";
import { useProcessActivity } from "../../lib/useAgentActivity";
import { useFloorStore } from "../../state/floorStore";
import { useProcessStore } from "../../state/processStore";
import { Bot, MiniBot, Monitor, TerminalDeskArt } from "./Bot";
import styles from "./Desk.module.css";

const MAX = 3;

/** Up to 3 sprites, then a "+N" counter. */
function Row({
  n,
  className,
  dim,
  item,
}: {
  n: number;
  className: string;
  dim?: boolean;
  item: (i: number) => ReactElement;
}) {
  if (n <= 0) return null;
  const shown = Math.min(n, MAX);
  return (
    <span className={className} data-dim={dim || undefined}>
      {Array.from({ length: shown }, (_, i) => item(i))}
      {n > MAX && <b className={styles.more}>+{n - MAX}</b>}
    </span>
  );
}

export function Desk({ process }: { process: ProcessInfo }) {
  const act = useProcessActivity(process.id);
  const active = useProcessStore((s) => s.activeProcessId === process.id);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (act.sinceMs === null) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 15_000);
    return () => clearInterval(t);
  }, [act.sinceMs]);

  const terminal = process.kind.kind === "terminal";
  const state = deskState(process, act);
  const since = formatSince(act.sinceMs, now);
  const label = terminal
    ? "terminal"
    : state === "ended" || state === "crashed"
      ? state
      : `${state === "needs" ? "waiting" : state}${since ? ` ${since}` : ""}`;
  // Counts are 0 for Windows, WSL and Auggie agents: nothing to show.
  const working = !terminal && state === "working";

  const open = () => {
    useProcessStore.getState().setActiveProcess(process.id);
    useFloorStore.getState().openTerminal(process.id);
  };
  const hover = (id: string | null) => () =>
    useFloorStore.getState().setHoverDesk(id);

  return (
    <button
      type="button"
      className={styles.desk}
      data-state={terminal ? "terminal" : state}
      data-working={working || undefined}
      data-active={active || undefined}
      style={
        {
          "--agent": process.color ?? "var(--text-faint)",
        } as React.CSSProperties
      }
      aria-label={`${process.name}, ${label}`}
      onClick={open}
      onMouseEnter={hover(process.id)}
      onMouseLeave={hover(null)}
      onFocus={hover(process.id)}
      onBlur={hover(null)}
    >
      {!terminal && (
        <Row
          n={act.shells}
          className={styles.shells}
          item={(i) => <Monitor key={i} seed={`${process.id}m${i}`} />}
        />
      )}
      <span className={styles.pad}>
        {terminal ? (
          <TerminalDeskArt />
        ) : (
          <Bot color={process.color} state={state} seed={process.id} />
        )}
        {state === "needs" && !terminal && (
          <span className={`${styles.mark} ${styles.ask}`} aria-hidden="true">
            ?
          </span>
        )}
        {state === "crashed" && !terminal && (
          <span className={`${styles.mark} ${styles.bang}`} aria-hidden="true">
            !
          </span>
        )}
      </span>
      {!terminal && (
        <Row
          n={act.subagents}
          className={styles.minis}
          dim={!working}
          item={(i) => (
            <MiniBot
              key={i}
              color={process.color}
              seed={`${process.id}s${i}`}
            />
          )}
        />
      )}
      <span className={styles.name}>{process.name}</span>
      <span className={styles.label}>{label}</span>
    </button>
  );
}
