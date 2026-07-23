/**
 * Archived scratchpads for one project, shown in a modal opened from the
 * Scratchpads submenu header. Each archived scratchpad can be restored
 * (unarchived) back into the active list or permanently deleted — deletion
 * lives here so a scratchpad must be archived before it can be removed (same
 * as to-dos). The list refreshes from the backend whenever the modal opens,
 * and reflects `scratchpad:changed` refreshes while open.
 */

import { useEffect } from "react";

import type { ProjectId } from "../ipc/types";
import { formatTime } from "../lib/dateFormat";
import { useScratchpadStore } from "../state/scratchpadStore";
import styles from "./ArchiveModal.module.css";
import { Modal } from "./Modal";
import { DeleteIcon, UnarchiveIcon } from "./icons";

const NO_SCRATCHPADS = [] as const;

export function ScratchpadArchiveModal({
  open,
  projectId,
  onClose,
}: {
  open: boolean;
  projectId: ProjectId;
  onClose: () => void;
}) {
  const archived = useScratchpadStore(
    (s) => s.archivedByProject[projectId] ?? NO_SCRATCHPADS,
  );
  const refreshArchived = useScratchpadStore((s) => s.refreshArchived);
  const setScratchpadArchived = useScratchpadStore(
    (s) => s.setScratchpadArchived,
  );
  const removeScratchpad = useScratchpadStore((s) => s.removeScratchpad);

  useEffect(() => {
    if (open) void refreshArchived(projectId);
  }, [open, projectId, refreshArchived]);

  return (
    <Modal
      open={open}
      title="Archived scratchpads"
      onClose={onClose}
      width={520}
    >
      {archived.length === 0 ? (
        <p className={styles.empty}>No archived scratchpads yet.</p>
      ) : (
        <ul className={styles.list}>
          {archived.map((sp) => (
            <li key={sp.id} className={styles.item}>
              <span className={styles.text} title={sp.title}>
                {sp.title}
              </span>
              <span className={styles.when}>{formatTime(sp.archivedAt)}</span>
              <button
                type="button"
                className={styles.restore}
                aria-label={`Restore "${sp.title}"`}
                title="Restore to the active list"
                onClick={() =>
                  void setScratchpadArchived(projectId, sp.id, false)
                }
              >
                <UnarchiveIcon size={13} />
                Restore
              </button>
              <button
                type="button"
                className={styles.remove}
                aria-label={`Delete "${sp.title}"`}
                title="Delete permanently"
                onClick={() => void removeScratchpad(projectId, sp.id)}
              >
                <DeleteIcon size={13} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </Modal>
  );
}
