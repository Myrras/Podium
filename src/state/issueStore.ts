/**
 * Per-project cache of the Git board (issues + PRs/MRs). Refreshing never
 * blocks the UI: one in-flight fetch per project, and a failed fetch keeps the
 * last board and only records the error, which the board shows inline (no
 * toasts).
 *
 * Nothing is fetched while Settings → Issues has no setup: `refresh` then
 * records `issuesNotConfigured` without calling the backend.
 *
 * Refresh strategy lives here, not in the backend: `startAutoRefresh` (called
 * while the project's Git tab is mounted) first shows the backend's cached
 * board, fetches at once only when that is missing or older than the poll
 * interval, then every 5 minutes, and on window focus when the last attempt
 * is older than a minute.
 */

import { create } from "zustand";

import {
  issuesBoard,
  issuesBoardCached,
  issuesBoardMove,
  issuesBoardUnassign,
  issuesSettingsGet,
  toIpcError,
} from "../ipc/commands";
import type {
  Board,
  BoardColumn,
  BoardItem,
  IpcError,
  IssuesOverview,
  ProjectId,
} from "../ipc/types";
import { toastError } from "./toastStore";

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

/** Same kind and text as the backend's `IssuesNotConfigured`. */
export const NOT_CONFIGURED: IpcError = {
  kind: "issuesNotConfigured",
  message: "No Git board is set up: add one in Settings → Issues.",
};

const EMPTY: ProjectBoard = {
  board: null,
  error: null,
  loading: false,
  fetchedAt: 0,
  mine: true,
};

/** Columns the item may be dropped on: they accept its kind and it is not there yet. */
export const moveTargets = (
  item: BoardItem,
  columns: BoardColumn[],
): BoardColumn[] =>
  item.movable
    ? columns.filter(
        (c) => c.accepts.includes(item.kind) && c.id !== item.column,
      )
    : [];

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
  /**
   * Show the backend's cached board for the current scope (no network).
   * Resolves to its fetch time in ms, or null when there is none.
   */
  hydrate: (projectId: ProjectId) => Promise<number | null>;
  /** Move a card: optimistic, reverted with a toast when the backend refuses. */
  move: (
    projectId: ProjectId,
    item: BoardItem,
    columnId: string,
  ) => Promise<void>;
  /** Unassign the signed-in user; a refusal becomes a toast. */
  unassign: (projectId: ProjectId, item: BoardItem) => Promise<void>;
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
      if (!get().overview) await get().loadOverview();
      const { overview } = get();
      // No setup in Settings → Issues: do not ask the backend at all, so
      // nothing shells out (git, gh) for a board nobody set up. An
      // unreadable overview is not proof of that, so the backend decides.
      if (overview?.setups?.length === 0) {
        patch(projectId, {
          board: null,
          error: NOT_CONFIGURED,
          loading: false,
          fetchedAt: Date.now(),
        });
        return;
      }
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

    hydrate: async (projectId) => {
      const mine = (get().byProject[projectId] ?? EMPTY).mine;
      try {
        const cached = await issuesBoardCached(projectId, mine);
        // The scope changed while reading: that answer is for the old one.
        if (!cached || (get().byProject[projectId]?.mine ?? true) !== mine) {
          return null;
        }
        const fetchedAt = Date.parse(cached.fetchedAt) || 0;
        patch(projectId, { board: cached.board, fetchedAt });
        return fetchedAt;
      } catch {
        return null; // The cache is only a head start.
      }
    },

    move: async (projectId, item, columnId) => {
      const prev = get().byProject[projectId]?.board;
      if (!prev) return;
      patch(projectId, {
        board: {
          ...prev,
          items: prev.items.map((i) =>
            i.kind === item.kind && i.number === item.number
              ? { ...i, column: columnId }
              : i,
          ),
        },
      });
      try {
        await issuesBoardMove(projectId, item.kind, item.number, columnId);
      } catch (e: unknown) {
        patch(projectId, { board: prev });
        void get().hydrate(projectId);
        toastError(toIpcError(e).message);
      }
    },

    unassign: async (projectId, item) => {
      try {
        // The board follows via `board:changed` (the backend updated its cache).
        await issuesBoardUnassign(projectId, item.kind, item.number);
      } catch (e: unknown) {
        toastError(toIpcError(e).message);
      }
    },

    setMine: (projectId, mine) => {
      patch(projectId, { mine });
      void get()
        .hydrate(projectId)
        .then(() => get().refresh(projectId));
    },

    startAutoRefresh: (projectId) => {
      const { refresh } = get();
      void get()
        .hydrate(projectId)
        .then((at) => {
          if (at === null || Date.now() - at > ISSUE_POLL_MS) {
            void refresh(projectId);
          }
        });
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
