/**
 * The assistant's sub-agents, pinned above the chat log: the work it handed
 * off so it could answer straight away. Per row the name and task (the full
 * task on hover), its kind, how long it has run and where it is; a running
 * or queued one can be stopped, and clicking a started one (or Enter/Space
 * on it) opens its terminal in the work area (for a lookup that terminal is
 * its output). Ended rows stay for a
 * few minutes (the backend drops them) with their result or error. Hidden
 * when there are none.
 */

import { useEffect, useState, type KeyboardEvent } from "react";

import type { SubagentInfo, SubagentPhase } from "../ipc/types";
import {
  formatElapsed,
  isSubagentLive,
  subagentElapsedMs,
  subagentSummary,
} from "../lib/subagents";
import { useAgentActivity } from "../lib/useAgentActivity";
import { useAssistantStore } from "../state/assistantStore";
import { useLayoutStore } from "../state/layoutStore";
import { StatusDot } from "./StatusDot";
import { CaretIcon, StopIcon } from "./icons";
import styles from "./SubagentsSection.module.css";

const PHASE_LABELS: Record<SubagentPhase, string> = {
  queued: "Queued",
  running: "Running",
  waiting: "Needs input",
  done: "Done",
  failed: "Failed",
  stopped: "Stopped",
};

export function SubagentsSection() {
  const subagents = useAssistantStore((s) => s.subagents);
  const [open, setOpen] = useState(true);
  // One clock for the whole section, ticking only while something runs, so
  // the running times stay live without a timer per row.
  const anyRunning = subagents.some(
    (a) => a.phase === "running" || a.phase === "waiting",
  );
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!anyRunning) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [anyRunning]);

  if (subagents.length === 0) return null;
  const summary = subagentSummary(subagents);

  return (
    <section className={styles.section} aria-label="Sub-agents">
      <button
        type="button"
        className={styles.header}
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <span className={styles.caret} data-open={open}>
          <CaretIcon />
        </span>
        Sub-agents
        {summary && <span className={styles.summary}>{summary}</span>}
      </button>
      {open && (
        <ul className={styles.list}>
          {subagents.map((a) => (
            <SubagentRow key={a.id} item={a} now={now} />
          ))}
        </ul>
      )}
    </section>
  );
}

function SubagentRow({ item, now }: { item: SubagentInfo; now: number }) {
  const stopSubagent = useAssistantStore((s) => s.stopSubagent);
  const openSubagent = useLayoutStore((s) => s.openSubagent);
  const openInWorkArea = useLayoutStore((s) => s.openSubagentInWorkArea);
  const activity = useAgentActivity(item.id);
  const running = item.phase === "running";
  const elapsed = subagentElapsedMs(item, now);
  const time =
    item.phase === "queued"
      ? "queued"
      : elapsed === null
        ? ""
        : formatElapsed(elapsed);
  const openable = item.startedAt !== null;
  const open = () => openInWorkArea(item.id);
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      open();
    }
  };
  // Only the task when the name already says the same thing.
  const showTask = item.task.trim() !== item.name.trim();

  // The outcome — or, while waiting, the question — stays out of the row;
  // hovering the phase shows it whole.
  const detail =
    (item.phase === "done"
      ? item.result
      : item.phase === "waiting"
        ? item.question
        : item.error) ?? undefined;

  return (
    <li
      className={styles.row}
      data-phase={item.phase}
      data-open={openSubagent === item.id || undefined}
    >
      <StatusDot
        status={item.status}
        activity={running ? activity : undefined}
      />
      {/* The row itself opens the terminal; a queued one has none yet. */}
      <div
        className={styles.body}
        role={openable ? "button" : undefined}
        tabIndex={openable ? 0 : undefined}
        aria-label={openable ? `Open ${item.name}` : undefined}
        data-openable={openable || undefined}
        onClick={openable ? open : undefined}
        onKeyDown={openable ? onKey : undefined}
      >
        <div className={styles.meta}>
          <span className={styles.name} title={item.task}>
            {item.name}
          </span>
          <span className={styles.kind} data-kind={item.kind}>
            {item.kind}
          </span>
          <span className={styles.phase} data-phase={item.phase} title={detail}>
            {PHASE_LABELS[item.phase]}
          </span>
          {time && <span className={styles.time}>{time}</span>}
        </div>
        {showTask && (
          <span className={styles.task} title={item.task}>
            {item.task}
          </span>
        )}
      </div>
      <div className={styles.actions}>
        {isSubagentLive(item) && (
          <button
            type="button"
            className={styles.iconBtn}
            aria-label={`Stop ${item.name}`}
            title={item.phase === "queued" ? "Cancel" : "Stop"}
            onClick={(e) => {
              // Never also opens the terminal.
              e.stopPropagation();
              void stopSubagent(item.id);
            }}
          >
            <StopIcon />
          </button>
        )}
      </div>
    </li>
  );
}
