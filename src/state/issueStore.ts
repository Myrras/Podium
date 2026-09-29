/**
 * Per-project cache of the user's open assigned issues (Issues sidebar
 * section). Refreshing never blocks the UI: one in-flight fetch per project,
 * and a failed fetch keeps the last list and only records the error, which
 * the section shows inline (no toasts).
 *
 * Refresh strategy lives here, not in the backend: `startAutoRefresh` (called
 * while a section is mounted) fetches immediately, every 5 minutes, and on
 * window focus when the last attempt is older than a minute.
 */

import { create } from "zustand";

import { issuesList, toIpcError } from "../ipc/commands";
import type { IpcError, IssueInfo, ProjectId } from "../ipc/types";

export const ISSUE_POLL_MS = 5 * 60_000;
export const ISSUE_FOCUS_STALE_MS = 60_000;

export interface ProjectIssues {
  issues: IssueInfo[];
  /** Set by the last fetch if it failed; cleared by the next success. */
  error: IpcError | null;
  loading: boolean;
  /** Time of the last fetch attempt (ms), 0 = never. */
  fetchedAt: number;
}

const EMPTY: ProjectIssues = {
  issues: [],
  error: null,
  loading: false,
  fetchedAt: 0,
};

interface IssueState {
  byProject: Record<ProjectId, ProjectIssues>;
  /** Fetch now; a no-op while this project already has one in flight. */
  refresh: (projectId: ProjectId) => Promise<void>;
  /** Fetch + poll + focus-refresh for as long as the returned stop fn is not called. */
  startAutoRefresh: (projectId: ProjectId) => () => void;
  /** Event applier for `project:closed`. */
  dropProject: (projectId: ProjectId) => void;
}

export const useIssueStore = create<IssueState>((set, get) => ({
  byProject: {},

  refresh: async (projectId) => {
    const current = get().byProject[projectId] ?? EMPTY;
    if (current.loading) return;
    const patch = (p: Partial<ProjectIssues>) =>
      set((s) => ({
        byProject: {
          ...s.byProject,
          [projectId]: { ...(s.byProject[projectId] ?? EMPTY), ...p },
        },
      }));
    patch({ loading: true });
    try {
      const issues = await issuesList(projectId);
      patch({ issues, error: null, loading: false, fetchedAt: Date.now() });
    } catch (e: unknown) {
      patch({ error: toIpcError(e), loading: false, fetchedAt: Date.now() });
    }
  },

  startAutoRefresh: (projectId) => {
    const { refresh } = get();
    void refresh(projectId);
    const timer = setInterval(() => void refresh(projectId), ISSUE_POLL_MS);
    const onFocus = () => {
      const at = get().byProject[projectId]?.fetchedAt ?? 0;
      if (Date.now() - at > ISSUE_FOCUS_STALE_MS) void refresh(projectId);
    };
    window.addEventListener("focus", onFocus);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", onFocus);
    };
  },

  dropProject: (projectId) =>
    set((s) => {
      const next = { ...s.byProject };
      delete next[projectId];
      return { byProject: next };
    }),
}));
