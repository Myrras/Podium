/** Small colored dot summarizing a process's lifecycle state. */

import type { ProcessStatus } from "../ipc/types";
import { statusLabel } from "../lib/processStatus";
import type { AgentActivity } from "../lib/useAgentActivity";
import styles from "./StatusDot.module.css";

export function StatusDot({
  status,
  activity,
}: {
  status: ProcessStatus;
  /**
   * Agent activity overlay (see useAgentActivity): pulses green while
   * "working", turns blue while "waiting" (needs input). Omit for non-agents.
   */
  activity?: AgentActivity;
}) {
  const label =
    activity === "waiting" ? "Needs your input" : statusLabel(status);
  return (
    <span
      className={styles.dot}
      data-state={status.state}
      data-working={activity === "working" ? "true" : undefined}
      data-waiting={activity === "waiting" ? "true" : undefined}
      data-crashed={
        status.state === "exited" && status.crashed ? "true" : undefined
      }
      title={label}
      aria-label={label}
      role="img"
    />
  );
}
