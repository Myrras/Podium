/** Work-area pane for the focused process: header bar + live terminal. */

import { useEffect, useState } from "react";

import { processGitBranch } from "../ipc/commands";
import type { ProcessInfo } from "../ipc/types";
import { activityLabel, isActive, statusLabel } from "../lib/processStatus";
import { useAgentActivity } from "../lib/useAgentActivity";
import { useProcessStore } from "../state/processStore";
import { StatusDot } from "./StatusDot";
import { TerminalView } from "./TerminalView";
import { BranchIcon, RestartIcon, RunIcon, StopIcon, TerminalIcon } from "./icons";
import styles from "./TerminalPane.module.css";

export function TerminalPane({ process }: { process: ProcessInfo }) {
  const startProcess = useProcessStore((s) => s.startProcess);
  const stopProcess = useProcessStore((s) => s.stopProcess);
  const restartProcess = useProcessStore((s) => s.restartProcess);

  const active = isActive(process.status);
  const isAgent = process.kind.kind === "agent";
  const running = process.status.state === "running";
  const activity = useAgentActivity(process.id);

  // Git branch of the focused process's cwd (null when not a git repo). Fetched
  // on demand — it shells out to git, so it's kept off the process_list path.
  const [branch, setBranch] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    processGitBranch(process.id)
      .then((b) => alive && setBranch(b))
      .catch(() => alive && setBranch(null));
    return () => {
      alive = false;
    };
  }, [process.id]);

  return (
    <div className={styles.pane}>
      <header className={styles.header}>
        <TerminalIcon className={styles.kindIcon} />
        <span className={styles.name}>{process.name}</span>
        <span className={styles.status}>
          <StatusDot
            status={process.status}
            activity={isAgent ? activity : undefined}
          />
          <span className={styles.statusText}>
            {statusLabel(process.status)}
          </span>
          {isAgent && running && (
            <span className={styles.activityText} data-activity={activity}>
              {activityLabel(activity)}
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
        {/* Agents get no start/stop/restart controls — an agent session is
            a one-shot conversation, not a restartable service. */}
        {!isAgent && (
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
      </header>
      <TerminalView processId={process.id} />
    </div>
  );
}
