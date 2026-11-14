/** Worktree removal shared by the sidebar and the Floor project menu. */

import { toIpcError, worktreeRemove } from "../ipc/commands";
import type { ProjectId } from "../ipc/types";
import { toastError } from "../state/toastStore";
import { useWorktreeStore } from "../state/worktreeStore";

/** The question asked before discarding a dirty worktree's changes. */
export const dirtyWorktreeQuestion = (name: string) =>
  `Worktree "${name}" has uncommitted changes. Discard them and remove it anyway?`;

/**
 * Remove a worktree and store the fresh list. A dirty one asks
 * `confirmForce` and retries with force on yes. Errors toast.
 * Resolves `true` when the worktree is gone.
 */
export async function removeWorktree(
  projectId: ProjectId,
  name: string,
  confirmForce: (name: string) => Promise<boolean>,
): Promise<boolean> {
  const { setWorktrees } = useWorktreeStore.getState();
  try {
    try {
      setWorktrees(projectId, await worktreeRemove(projectId, name, false));
      return true;
    } catch (e: unknown) {
      const err = toIpcError(e);
      if (err.kind !== "worktreeDirty") {
        toastError("Could not remove worktree", err.message);
        return false;
      }
      if (!(await confirmForce(name))) return false;
      setWorktrees(projectId, await worktreeRemove(projectId, name, true));
      return true;
    }
  } catch (e2: unknown) {
    toastError("Could not remove worktree", toIpcError(e2).message);
    return false;
  }
}
