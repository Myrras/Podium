/** Work-area pane for the focused process: header bar + live terminal. */

import type { ProcessInfo } from "../ipc/types";
import { activityLabel, isActive, statusLabel } from "../lib/processStatus";
import { useAgentActivity } from "../lib/useAgentActivity";
import { useProcessStore } from "../state/processStore";
import { StatusDot } from "./StatusDot";
import { TerminalView } from "./TerminalView";
import { RestartIcon, RunIcon, StopIcon, TerminalIcon } from "./icons";
import styles from "./TerminalPane.module.css";

export function TerminalPane({ process }: { process: ProcessInfo }) {
  const startProcess = useProcessStore((s) => s.startProcess);
  const stopProcess = useProcessStore((s) => s.stopProcess);
  const restartProcess = useProcessStore((s) => s.restartProcess);

  const active = isActive(process.status);
  const isAgent = process.kind.kind === "agent";
  const running = process.status.state === "running";
  const activity = useAgentActivity(process.id);

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
