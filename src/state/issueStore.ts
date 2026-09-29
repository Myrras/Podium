/**
 * Per-project cache of the Git board (issues + PRs/MRs). Refreshing never
 * blocks the UI: one in-flight fetch per project, and a failed fetch keeps the
 * last board and only records the error, which the board shows inline (no
 * toasts).
 *
 * Refresh strategy lives here, not in the backend: `startAutoRefresh` (called
 * while the project's Git tab is mounted) fetches immediately, every 5
 * minutes, and on window focus when the last attempt is older than a minute.
 */

import { create } from "zustand";

import { issuesBoard, issuesSettingsGet, toIpcError } from "../ipc/commands";
import type { Board, IpcError, IssuesOverview, ProjectId } from "../ipc/types";

export const ISSUE_POLL_MS = 5 * 60_000;
export const ISSUE_FOCUS_STALE_MS = 60_000;

export interface ProjectBoard {
  board: Board | null;
  /** Set by the last fetch if it failed; cleared by the next success. */
  error: IpcError | null;
  loading: boolean;
  /** Time of the last fetch attempt (ms), 0 = never. */
  fetchedAt: number;
  /** Only the user's own issues/PRs (default) or everything in the repo. */
  mine: boolean;
}

const EMPTY: ProjectBoard = {
  board: null,
  error: null,
  loading: false,
  fetchedAt: 0,
  mine: true,
};

interface IssueState {
  byProject: Record<ProjectId, ProjectBoard>;
  /** Settings → Issues setups + per-project picks; `null` until loaded. */
  overview: IssuesOverview | null;
  /** Load `overview` once (a no-op when already loaded). */
  loadOverview: () => Promise<void>;
  /** Apply a changed overview and refetch every shown project with it. */
  setOverview: (overview: IssuesOverview) => void;
  /** Fetch now; a no-op while this project already has one in flight. */
  refresh: (projectId: ProjectId) => Promise<void>;
  /** Switch between the user's own items and the whole repo, then refetch. */
  setMine: (projectId: ProjectId, mine: boolean) => void;
  /** Fetch + poll + focus-refresh for as long as the returned stop fn is not called. */
  startAutoRefresh: (projectId: ProjectId) => () => void;
  /** Event applier for `project:closed`. */
  dropProject: (projectId: ProjectId) => void;
}

export const useIssueStore = create<IssueState>((set, get) => {
  const patch = (projectId: ProjectId, p: Partial<ProjectBoard>) =>
    set((s) => ({
      byProject: {
        ...s.byProject,
        [projectId]: { ...(s.byProject[projectId] ?? EMPTY), ...p },
      },
    }));

  return {
    byProject: {},
    overview: null,

    loadOverview: async () => {
      if (get().overview) return;
      try {
        const overview = await issuesSettingsGet();
        set({ overview });
      } catch {
        // The picker just stays hidden; fetching the board still works.
      }
    },

    setOverview: (overview) => {
      set({ overview });
      for (const projectId of Object.keys(get().byProject)) {
        void get().refresh(projectId);
      }
    },

    refresh: async (projectId) => {
      const current = get().byProject[projectId] ?? EMPTY;
      if (current.loading) return;
      const { mine } = current;
      patch(projectId, { loading: true });
      try {
        const board = await issuesBoard(projectId, mine);
        patch(projectId, {
          board,
          error: null,
          loading: false,
          fetchedAt: Date.now(),
        });
      } catch (e: unknown) {
        patch(projectId, {
          error: toIpcError(e),
          loading: false,
          fetchedAt: Date.now(),
        });
      }
      // The scope was switched mid-fetch: that answer was for the old one.
      if ((get().byProject[projectId]?.mine ?? true) !== mine) {
        await get().refresh(projectId);
      }
    },

    setMine: (projectId, mine) => {
      patch(projectId, { mine });
      void get().refresh(projectId);
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
  };
});
