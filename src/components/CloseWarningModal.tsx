/**
 * Warns before Podium quits while agents or terminals are still running.
 *
 * Raised by the backend's close/exit guard (via the `window:close-requested`
 * event): it lists every running agent/terminal so you can see what a quit
 * would interrupt, then "Close anyway" confirms via `windowConfirmClose`
 * (which arms the backend force-close flag and exits).
 */

import { useState } from "react";

import { toIpcError, windowConfirmClose } from "../ipc/commands";
import type { ProcessKind } from "../ipc/types";
import { isActive } from "../lib/processStatus";
import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";
import { toastError } from "../state/toastStore";
import { Modal } from "./Modal";
import { StatusDot } from "./StatusDot";
import styles from "./CloseWarningModal.module.css";

function kindLabel(kind: ProcessKind): string {
  return kind.kind === "agent" ? `Agent · ${kind.adapter}` : "Terminal";
}

export function CloseWarningModal({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const processes = useProcessStore((s) => s.processes);
  const projects = useProjectStore((s) => s.projects);
  const [busy, setBusy] = useState(false);

  // Only agents and terminals gate the close (services are `podium.yml`-owned
  // and meant to come and go with the app), matching the backend guard.
  const active = processes.filter(
    (p) =>
      isActive(p.status) &&
      (p.kind.kind === "agent" || p.kind.kind === "terminal"),
  );

  const projectName = (id: string) =>
    projects.find((p) => p.id === id)?.name ?? "Unknown project";

  const intro =
    active.length === 1
      ? "Closing Podium will stop this process. Any in-progress work will be interrupted."
      : active.length > 1
        ? `Closing Podium will stop these ${active.length} processes. Any in-progress work will be interrupted.`
        : "Podium may still have active processes. Any in-progress work will be interrupted.";

  const confirm = async () => {
    setBusy(true);
    try {
      await windowConfirmClose();
      // On success the app exits, so nothing more to do; if it somehow returns
      // (see catch), the user can retry.
    } catch (e) {
      setBusy(false);
      toastError("Could not close Podium", toIpcError(e).message);
    }
  };

  return (
    <Modal
      open={open}
      title="Active processes still running"
      tone="warning"
      onClose={onClose}
      footer={
        <>
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="danger"
            disabled={busy}
            onClick={() => void confirm()}
          >
            {busy ? "Closing…" : "Close anyway"}
          </button>
        </>
      }
    >
      <p className={styles.intro}>{intro}</p>
      {active.length > 0 && (
        <ul className={styles.list}>
          {active.map((p) => (
            <li key={p.id} className={styles.item}>
              <StatusDot status={p.status} />
              <span className={styles.itemText}>
                <span className={styles.name}>{p.name}</span>
                <span className={styles.meta}>
                  {projectName(p.projectId)} · {kindLabel(p.kind)}
                </span>
              </span>
            </li>
          ))}
        </ul>
      )}
    </Modal>
  );
}
