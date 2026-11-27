/** Work-area pane for the focused process: header bar + live terminal. */

import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";

import { processGitBranch } from "../ipc/commands";
import type { ProcessId, ProcessInfo } from "../ipc/types";
import { isActive, pillState, statusLabel } from "../lib/processStatus";
import { useProcessActivity } from "../lib/useAgentActivity";
import { useAssistantStore } from "../state/assistantStore";
import { useProcessStore } from "../state/processStore";
import { StatusPill } from "./StatusPill";
import { TerminalView } from "./TerminalView";
import {
  BranchIcon,
  RestartIcon,
  RunIcon,
  StopIcon,
  TerminalIcon,
} from "./icons";
import styles from "./TerminalPane.module.css";

// Last-known git branch per process, surviving pane unmounts. Switching to a
// window shows the cached branch instantly (no flicker to empty) while a
// background refresh updates it — the git call itself takes a login shell.
const branchCache = new Map<ProcessId, string | null>();

export function TerminalPane({
  process,
  leading,
  trailing,
  onRename,
  hideActions = false,
}: {
  process: ProcessInfo;
  /** Replaces the terminal icon at the far left. */
  leading?: ReactNode;
  /** Rendered at the far right of the header. */
  trailing?: ReactNode;
  /** Makes the name editable inline; a blank name comes through as null. */
  onRename?: (name: string | null) => void;
  /** Hides the built-in Stop/Start/Restart buttons. */
  hideActions?: boolean;
}) {
  const startProcess = useProcessStore((s) => s.startProcess);
  const stopProcess = useProcessStore((s) => s.stopProcess);
  const restartProcess = useProcessStore((s) => s.restartProcess);
  const stopSubagent = useAssistantStore((s) => s.stopSubagent);

  const active = isActive(process.status);
  const isAgent = process.kind.kind === "agent";
  // Also covers an assistant sub-agent: the monitor tracks those in `details`.
  const activity = useProcessActivity(process.id);
  const state = pillState(process.status, process.kind.kind, activity);

  // One of the assistant's sub-agents: stopped through its own path (the
  // assistant is told), and only while it still holds or waits for a slot.
  const subagent = process.subagent;
  const subagentLive =
    subagent?.phase === "queued" ||
    subagent?.phase === "running" ||
    subagent?.phase === "waiting";

  // Git branch of the focused process's cwd (null when not a git repo). Fetched
  // on demand — it shells out to git, so it's kept off the process_list path.
  // Seed from the cache so re-focusing a window shows the branch immediately.
  const [branch, setBranch] = useState<string | null>(
    () => branchCache.get(process.id) ?? null,
  );
  useEffect(() => {
    let alive = true;
    setBranch(branchCache.get(process.id) ?? null);
    processGitBranch(process.id)
      .then((b) => {
        branchCache.set(process.id, b);
        if (alive) setBranch(b);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [process.id]);

  const [draft, setDraft] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const editing = draft !== null;
  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);
  // The open edit's name, null once committed or cancelled. Each edit commits
  // at most once: Chromium blurs the input as Enter/Esc removes it. Closing
  // the Floor modal blurs it first, so that commit is the blur's.
  const pendingName = useRef<string | null>(null);
  const commitPending = () => {
    const name = pendingName.current;
    pendingName.current = null;
    // A process removed meanwhile (e.g. closed from the modal) has nothing to rename.
    const exists = useProcessStore
      .getState()
      .processes.some((p) => p.id === process.id);
    if (name === null || !exists) return;
    const next = name.trim();
    if (next !== process.name) onRename?.(next || null);
  };
  const commit = () => {
    setDraft(null);
    commitPending();
  };

  return (
    <div className={styles.pane}>
      <header
        className={
          trailing ? `${styles.header} ${styles.slotted}` : styles.header
        }
      >
        {leading ?? <TerminalIcon className={styles.kindIcon} />}
        {editing ? (
          <input
            ref={inputRef}
            className={styles.nameInput}
            aria-label="Name"
            value={draft}
            onChange={(e) => {
              setDraft(e.target.value);
              pendingName.current = e.target.value;
            }}
            onBlur={commit}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                commit();
              } else if (e.key === "Escape") {
                // Cancels the edit only: it must not close a modal around us.
                e.preventDefault();
                e.stopPropagation();
                pendingName.current = null; // a later blur saves nothing
                setDraft(null);
              }
            }}
          />
        ) : onRename ? (
          <button
            type="button"
            className={`${styles.name} ${styles.nameBtn}`}
            title="Rename"
            onClick={() => {
              pendingName.current = process.name;
              setDraft(process.name);
            }}
          >
            {process.name}
          </button>
        ) : (
          <span className={styles.name}>{process.name}</span>
        )}
        <span className={styles.status}>
          <StatusPill
            state={state}
            kind={process.kind.kind}
            sinceMs={activity.sinceMs}
            subagents={activity.subagents}
            shells={activity.shells}
          />
          <span className={styles.statusText}>
            {statusLabel(process.status)}
          </span>
          {state === "needs" && activity.waitingFor && (
            <span className={styles.waitingFor} title={activity.waitingFor}>
              {activity.waitingFor}
            </span>
          )}
        </span>
        {/* Git branch + worktree, shown top-right where agent controls would
            be. Only when the cwd is a git repo. */}
        {branch && (
          <span
            className={styles.gitInfo}
            title={
              process.worktree
                ? `On branch ${branch} in worktree ${process.worktree}`
                : `On branch ${branch}`
            }
          >
            <BranchIcon size={13} />
            <span className={styles.gitBranch}>{branch}</span>
            {process.worktree && (
              <span className={styles.gitWorktree}>{process.worktree}</span>
            )}
          </span>
        )}
        {subagent && (
          <span className={styles.subagentKind}>
            Assistant sub-agent · {subagent.kind}
          </span>
        )}
        {subagentLive && (
          <span className={styles.headerActions}>
            <button
              type="button"
              className={styles.headerBtn}
              aria-label={`Stop ${process.name}`}
              title="Stop this sub-agent"
              onClick={() => void stopSubagent(process.id)}
            >
              <StopIcon />
            </button>
          </span>
        )}
        {/* Agents get no start/stop/restart controls — an agent session is
            a one-shot conversation, not a restartable service. */}
        {!isAgent && !hideActions && (
          <span className={styles.headerActions}>
            {active ? (
              <button
                type="button"
                className={styles.headerBtn}
                aria-label={`Stop ${process.name}`}
                title="Stop"
                onClick={() => void stopProcess(process.id)}
              >
                <StopIcon />
              </button>
            ) : (
              <button
                type="button"
                className={styles.headerBtn}
                aria-label={`Start ${process.name}`}
                title="Start"
                onClick={() => void startProcess(process.id)}
              >
                <RunIcon />
              </button>
            )}
            <button
              type="button"
              className={styles.headerBtn}
              aria-label={`Restart ${process.name}`}
              title="Restart"
              onClick={() => void restartProcess(process.id)}
            >
              <RestartIcon />
            </button>
          </span>
        )}
        {trailing && <span className={styles.trailing}>{trailing}</span>}
      </header>
      <TerminalView processId={process.id} />
    </div>
  );
}
