/**
 * Podium-managed git worktrees for one project, opened from the Agents
 * subsection header. The list is fetched fresh on every open (git is the
 * source of truth — no events). A worktree in use by a running process can't
 * be deleted; a dirty one asks for confirmation before a force removal.
 */

import { useEffect, useState } from "react";

import { toIpcError, worktreeList, worktreeRemove } from "../ipc/commands";
import type { ProjectId, WorktreeInfo } from "../ipc/types";
import { toastError } from "../state/toastStore";
import { Modal } from "./Modal";
import { BranchIcon, DeleteIcon } from "./icons";
import styles from "./WorktreesModal.module.css";

export function WorktreesModal({
  open,
  projectId,
  onClose,
}: {
  open: boolean;
  projectId: ProjectId;
  onClose: () => void;
}) {
  /** `null` while loading. */
  const [worktrees, setWorktrees] = useState<WorktreeInfo[] | null>(null);
  /** Name of the worktree whose removal is in flight — git can take a moment,
   * so its delete button is disabled until it resolves (no button spamming). */
  const [removing, setRemoving] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setWorktrees(null);
    worktreeList(projectId)
      .then((list) => {
        if (!cancelled) setWorktrees(list);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setWorktrees([]);
        toastError("Could not list worktrees", toIpcError(e).message);
      });
    return () => {
      cancelled = true;
    };
  }, [open, projectId]);

  const handleDelete = async (wt: WorktreeInfo) => {
    if (removing) return;
    setRemoving(wt.name);
    try {
      try {
        setWorktrees(await worktreeRemove(projectId, wt.name, false));
      } catch (e: unknown) {
        const err = toIpcError(e);
        if (err.kind !== "worktreeDirty") {
          toastError("Could not remove worktree", err.message);
          return;
        }
        const discard = window.confirm(
          `Worktree "${wt.name}" has uncommitted changes. Discard them and remove it anyway?`,
        );
        if (!discard) return;
        setWorktrees(await worktreeRemove(projectId, wt.name, true));
      }
    } catch (e2: unknown) {
      toastError("Could not remove worktree", toIpcError(e2).message);
    } finally {
      setRemoving(null);
    }
  };

  return (
    <Modal open={open} title="Worktrees" onClose={onClose} width={520}>
      {worktrees === null ? (
        <p className={styles.empty}>Loading…</p>
      ) : worktrees.length === 0 ? (
        <p className={styles.empty}>
          No worktrees yet. Spawn an agent with "Run in a git worktree" to
          create one.
        </p>
      ) : (
        <ul className={styles.list}>
          {worktrees.map((wt) => (
            <li key={wt.name} className={styles.item}>
              <BranchIcon className={styles.branchIcon} />
              <span className={styles.name} title={wt.path}>
                {wt.name}
              </span>
              <span className={styles.branch}>{wt.branch}</span>
              {wt.inUse && <span className={styles.inUse}>in use</span>}
              <button
                type="button"
                className={styles.remove}
                aria-label={`Delete worktree ${wt.name}`}
                title={
                  wt.inUse
                    ? "A process is still running in this worktree"
                    : removing === wt.name
                      ? "Removing…"
                      : "Delete worktree"
                }
                data-busy={removing === wt.name || undefined}
                disabled={wt.inUse || removing !== null}
                onClick={() => void handleDelete(wt)}
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
