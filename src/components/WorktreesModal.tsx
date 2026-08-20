/**
 * Podium-managed git worktrees for one project, opened from the Agents
 * subsection header. Reads from the shared worktree store (backed by
 * `git worktree list --porcelain`, cached and kept fresh by
 * `worktree:changed`), so an open with a warm cache shows the list
 * immediately; opening still kicks off a background refresh to catch
 * out-of-band changes (e.g. a worktree removed from a terminal). A worktree
 * in use by a running process can't be deleted; a dirty one asks for
 * confirmation before a force removal.
 */

import { useEffect, useState } from "react";

import { toIpcError, worktreeRemove } from "../ipc/commands";
import type { ProjectId, WorktreeInfo } from "../ipc/types";
import { useWorktreeStore } from "../state/worktreeStore";
import { toastError } from "../state/toastStore";
import { Modal } from "./Modal";
import { BranchIcon, DeleteIcon, TerminalIcon } from "./icons";
import styles from "./WorktreesModal.module.css";

export function WorktreesModal({
  open,
  projectId,
  onClose,
  onOpenTerminal,
}: {
  open: boolean;
  projectId: ProjectId;
  onClose: () => void;
  /** Open a new terminal rooted in this worktree; closes the modal after. */
  onOpenTerminal: (worktree: WorktreeInfo) => void;
}) {
  const worktrees = useWorktreeStore(
    (s) => s.worktreesByProject[projectId] ?? null,
  );
  const refresh = useWorktreeStore((s) => s.refresh);
  const setWorktrees = useWorktreeStore((s) => s.setWorktrees);
  /** Name of the worktree whose removal is in flight — git can take a moment,
   * so its delete button is disabled until it resolves (no button spamming). */
  const [removing, setRemoving] = useState<string | null>(null);

  useEffect(() => {
    if (open) void refresh(projectId);
  }, [open, projectId, refresh]);

  const handleDelete = async (wt: WorktreeInfo) => {
    if (removing) return;
    setRemoving(wt.name);
    try {
      try {
        setWorktrees(
          projectId,
          await worktreeRemove(projectId, wt.name, false),
        );
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
        setWorktrees(projectId, await worktreeRemove(projectId, wt.name, true));
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
                className={styles.iconButton}
                aria-label={`Open terminal in ${wt.name}`}
                title="Open terminal here"
                onClick={() => onOpenTerminal(wt)}
              >
                <TerminalIcon size={13} />
              </button>
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
