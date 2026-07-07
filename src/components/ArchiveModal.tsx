/**
 * Archived to-dos for one project, shown in a modal opened from the To-dos
 * submenu header. Each archived to-do can be restored (unarchived) back into
 * the active list or permanently deleted — deletion lives here so a to-do must
 * be archived before it can be removed. The list refreshes from the backend
 * whenever the modal opens, and reflects `todo:changed` refreshes while open.
 */

import { useEffect } from "react";

import type { ProjectId } from "../ipc/types";
import { useTodoStore } from "../state/todoStore";
import styles from "./ArchiveModal.module.css";
import { Modal } from "./Modal";
import { CheckIcon, DeleteIcon, UnarchiveIcon } from "./icons";

const NO_TODOS = [] as const;

/** Compact local-time label (e.g. "Apr 3, 14:05"). */
function formatTime(iso: string | null): string {
  if (!iso) return "";
  return new Date(iso).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function ArchiveModal({
  open,
  projectId,
  onClose,
}: {
  open: boolean;
  projectId: ProjectId;
  onClose: () => void;
}) {
  const archived = useTodoStore(
    (s) => s.archivedByProject[projectId] ?? NO_TODOS,
  );
  const refreshArchived = useTodoStore((s) => s.refreshArchived);
  const setTodoArchived = useTodoStore((s) => s.setTodoArchived);
  const removeTodo = useTodoStore((s) => s.removeTodo);

  useEffect(() => {
    if (open) void refreshArchived(projectId);
  }, [open, projectId, refreshArchived]);

  return (
    <Modal open={open} title="Archived to-dos" onClose={onClose} width={520}>
      {archived.length === 0 ? (
        <p className={styles.empty}>No archived to-dos yet.</p>
      ) : (
        <ul className={styles.list}>
          {archived.map((todo) => (
            <li key={todo.id} className={styles.item}>
              {todo.done && <CheckIcon size={13} className={styles.doneIcon} />}
              <span
                className={styles.text}
                data-done={todo.done ? "true" : undefined}
                title={todo.text}
              >
                {todo.text}
              </span>
              <span className={styles.when}>{formatTime(todo.archivedAt)}</span>
              <button
                type="button"
                className={styles.restore}
                aria-label={`Restore "${todo.text}"`}
                title="Restore to the active list"
                onClick={() => void setTodoArchived(projectId, todo.id, false)}
              >
                <UnarchiveIcon size={13} />
                Restore
              </button>
              <button
                type="button"
                className={styles.remove}
                aria-label={`Delete "${todo.text}"`}
                title="Delete permanently"
                onClick={() => void removeTodo(projectId, todo.id)}
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
