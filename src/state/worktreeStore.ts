/**
 * Per-project Podium-managed git worktree lists, keyed by project id.
 *
 * `git worktree list` is still the source of truth (no local mutation
 * tracking), but the cache lets callers (the Worktrees modal, the Commands
 * modal's target picker) render instantly instead of showing "Loading…"
 * every time. `refresh` re-pulls from the backend; the `worktree:changed`
 * event (create/remove, from this UI or an agent over MCP) triggers it.
 */

import { create } from "zustand";

import { worktreeList } from "../ipc/commands";
import type { ProjectId, WorktreeInfo } from "../ipc/types";

interface WorktreeState {
  worktreesByProject: Record<ProjectId, WorktreeInfo[]>;
  /** Re-pull one project's worktree list (background load + `worktree:changed`). */
  refresh: (projectId: ProjectId) => Promise<void>;
  /** Replace one project's cached list (e.g. after a delete that returns the fresh list). */
  setWorktrees: (projectId: ProjectId, worktrees: WorktreeInfo[]) => void;
  /** Event applier for `project:closed` — drops the cached list. */
  dropProject: (projectId: ProjectId) => void;
}

export const useWorktreeStore = create<WorktreeState>((set) => ({
  worktreesByProject: {},

  refresh: async (projectId) => {
    let worktrees: WorktreeInfo[];
    try {
      worktrees = await worktreeList(projectId);
    } catch {
      // Silent by design: this runs in the background on every project open,
      // and a project that simply isn't a git repo would toast on each open.
      // Cache an empty list so the modal shows its empty state instead of
      // hanging on "Loading…"; the mutating actions toast their own errors.
      worktrees = [];
    }
    set((s) => ({
      worktreesByProject: { ...s.worktreesByProject, [projectId]: worktrees },
    }));
  },

  setWorktrees: (projectId, worktrees) =>
    set((s) => ({
      worktreesByProject: { ...s.worktreesByProject, [projectId]: worktrees },
    })),

  dropProject: (projectId) =>
    set((s) => {
      const next = { ...s.worktreesByProject };
      delete next[projectId];
      return { worktreesByProject: next };
    }),
}));
