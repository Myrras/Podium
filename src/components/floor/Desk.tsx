import type { ReactElement } from "react";
import type { ProcessInfo } from "../../ipc/types";
import { deskState, overlayOf } from "../../lib/floor/rooms";
import { formatSince, pillState } from "../../lib/processStatus";
import { useProcessActivity } from "../../lib/useAgentActivity";
import { followCamera } from "../../lib/floor/follow";
import { useContextMenu } from "../../lib/useContextMenu";
import { useNow } from "../../lib/useNow";
import { useFloorStore } from "../../state/floorStore";
import { useAssistantStore } from "../../state/assistantStore";
import { useProcessStore } from "../../state/processStore";
import { DESK_H } from "../../lib/floor/geometry";
import { openProcess, processMenu } from "../../lib/floor/menus";
import { Bot, MiniBot, Monitor, TerminalDeskArt } from "./Bot";
import styles from "./Desk.module.css";

const MAX = 3;

/** Up to 3 sprites, then a "+N" counter. */
function Row({
  n,
  className,
  dim,
  title,
  item,
}: {
  n: number;
  className: string;
  dim?: boolean;
  title: string;
  item: (i: number) => ReactElement;
}) {
  if (n <= 0) return null;
  const shown = Math.min(n, MAX);
  return (
    <span className={className} data-dim={dim || undefined} title={title}>
      {Array.from({ length: shown }, (_, i) => item(i))}
      {n > MAX && <b className={styles.more}>+{n - MAX}</b>}
    </span>
  );
}

/** `variant`: the agent's bot design, from `RoomModel.variants`. */
export function Desk({
  process,
  variant = 0,
}: {
  process: ProcessInfo;
  variant?: number;
}) {
  const act = useProcessActivity(process.id);
  const active = useProcessStore((s) => s.activeProcessId === process.id);
  const overlay = useAssistantStore((s) =>
    process.kind.kind === "agent" ? overlayOf(s.responses, process.id) : null,
  );
  const ringed = useFloorStore((s) => s.hoverDesk === process.id);
  const now = useNow(act.sinceMs, 15_000);

  const terminal = process.kind.kind === "terminal";
  const parked = overlay === "parked";
  // A parked agent shows the asleep bot, whatever it is really doing.
  const state = parked ? "idle" : deskState(process, act);
  // The live state is the truth: review shows only when nothing needs you or crashed.
  const review =
    overlay === "review" && state !== "needs" && state !== "crashed";
  const shown = parked ? "parked" : review ? "review" : null;
  const ps = pillState(process.status, process.kind.kind, act);
  const stopped =
    terminal && (ps === "exited" || ps === "crashed" || ps === "notStarted");
  const menu = useContextMenu(followCamera);
  const since = formatSince(act.sinceMs, now);
  const label = terminal
    ? stopped
      ? "stopped"
      : "terminal"
    : parked
      ? "parked"
      : review
        ? "ready to review"
        : state === "ended" || state === "crashed"
          ? state
          : `${state === "needs" ? "waiting" : state}${since ? ` ${since}` : ""}`;
  // Counts are 0 for Windows, WSL and Auggie agents: nothing to show.
  const working = !terminal && state === "working" && !parked;

  const hover = (id: string | null) => () =>
    useFloorStore.getState().setHoverDesk(id);

  return (
    <>
      <button
        type="button"
        className={styles.desk}
        data-state={terminal ? "terminal" : state}
        data-overlay={shown ?? undefined}
        data-ring={ringed || undefined}
        data-working={working || undefined}
        data-active={active || undefined}
        data-stopped={stopped || undefined}
        style={
          {
            height: DESK_H,
            "--agent": process.color ?? "var(--text-faint)",
          } as React.CSSProperties
        }
        aria-label={`${process.name}, ${label}`}
        title={`${process.name}, ${label}`}
        onClick={() => openProcess(process.id)}
        onContextMenu={(e) => menu.open(e, processMenu(process, act))}
        onMouseEnter={hover(process.id)}
        onMouseLeave={hover(null)}
        onFocus={hover(process.id)}
        onBlur={hover(null)}
      >
        {!terminal && (
          <Row
            n={act.shells}
            title={`${act.shells} background ${act.shells === 1 ? "shell" : "shells"}`}
            className={styles.shells}
            item={(i) => <Monitor key={i} seed={`${process.id}m${i}`} />}
          />
        )}
        <span className={styles.pad}>
          {terminal ? (
            <TerminalDeskArt />
          ) : (
            <Bot
              color={process.color}
              state={state}
              seed={process.id}
              variant={variant}
            />
          )}
          {state === "idle" && !terminal && !parked && (
            <span className={styles.zzz} aria-hidden="true">
              <i>z</i>
              <i>z</i>
              <i>Z</i>
            </span>
          )}
          {parked && !terminal && (
            <span
              className={`${styles.mark} ${styles.hollow}`}
              aria-hidden="true"
            >
              ?
            </span>
          )}
          {review && !terminal && (
            <span
              className={`${styles.mark} ${styles.check}`}
              aria-hidden="true"
            >
              ✓
            </span>
          )}
          {state === "needs" && !terminal && (
            <span className={`${styles.mark} ${styles.ask}`} aria-hidden="true">
              ?
            </span>
          )}
          {(terminal ? stopped && ps === "crashed" : state === "crashed") && (
            <span
              className={`${styles.mark} ${styles.bang}`}
              aria-hidden="true"
            >
              !
            </span>
          )}
        </span>
        {!terminal && (
          <Row
            n={act.subagents}
            title={`${act.subagents} ${act.subagents === 1 ? "sub-agent" : "sub-agents"}`}
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
      {menu.element}
    </>
  );
}
