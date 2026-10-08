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
import { useProjectStore } from "./projectStore";
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

/**
 * Columns the item may be dropped on: they accept its kind and it is not
 * there yet. Without project columns (`source` null) Open is only reachable
 * from Done — the backend's `validate_move` rule.
 */
export const moveTargets = (
  item: BoardItem,
  columns: BoardColumn[],
  source: string | null,
): BoardColumn[] =>
  item.movable
    ? columns.filter(
        (c) =>
          c.accepts.includes(item.kind) &&
          c.id !== item.column &&
          !(source === null && c.id === "open" && item.column !== "done"),
      )
    : [];

interface IssueState {
  byProject: Record<ProjectId, ProjectBoard>;
  /** Settings → Issues setups + per-project picks; `null` until loaded. */
  overview: IssuesOverview | null;
  /** Load `overview` once (a no-op when already loaded). */
  loadOverview: () => Promise<void>;
  /**
   * Apply a changed overview and refetch the projects whose Git board is
   * shown (paced like any automatic fetch).
   */
  setOverview: (overview: IssuesOverview) => void;
  /**
   * Fetch now; a no-op while this project already has one in flight, unless
   * `rerun` (the setups changed): then it runs once more after that one.
   * `auto` (poll, focus, mount) waits for its slot in the shared pacing.
   */
  refresh: (
    projectId: ProjectId,
    opts?: { rerun?: boolean; auto?: boolean },
  ) => Promise<void>;
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

/**
 * Automatic board fetches (poll, focus, mount) of all projects start at least
 * this far apart. One GitHub board costs ~6 search queries and GitHub allows
 * 30 a minute, so a focus refreshing six projects at once ran out; 15 s keeps
 * it at ~24 a minute. A refresh the user asks for skips the queue.
 */
// ponytail: one global pace for GitHub and GitLab alike; per-provider pacing
// if GitLab boards ever feel slow behind it.
export const AUTO_FETCH_SPACING_MS = 15_000;
let nextAutoFetch = 0;
/** Claims the next slot; a promise to wait for, or null when it is now. */
const autoFetchSlot = (): Promise<void> | null => {
  const now = Date.now();
  const at = Math.max(now, nextAutoFetch);
  nextAutoFetch = at + AUTO_FETCH_SPACING_MS;
  return at > now
    ? new Promise((resolve) => setTimeout(resolve, at - now))
    : null;
};
/** Tests only: forget the pacing between tests. */
export const resetAutoFetchPacing = () => {
  nextAutoFetch = 0;
  autoWaiting.clear();
};

/** How often each project was dropped, so a late answer can tell. */
const drops = new Map<ProjectId, number>();

/** Auto refreshes waiting for their slot, by project; a ticket per wait. */
const autoWaiting = new Map<ProjectId, object>();

/**
 * Errors no fetch can fix until something changes locally (a setup, the
 * remote): such a project's auto refresh costs no API call worth pacing, so
 * it takes no slot.
 */
const UNPACED_ERRORS = new Set([
  "issuesNotConfigured",
  "issuesNoRemote",
  "issuesUnsupportedHost",
  "issuesHostMismatch",
  "issuesGitlabNotConfigured",
  "issuesGhUnavailable",
]);
const unpaced = (error: IpcError | null) =>
  error !== null && UNPACED_ERRORS.has(error.kind);

/** Projects whose in-flight fetch must be followed by one more (`rerun`). */
const rerunAfterFetch = new Set<ProjectId>();

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
      // Only boards someone can see: a hidden or closed project's entry is
      // refetched by its own mount when it comes back.
      const shown = new Set(
        useProjectStore
          .getState()
          .projects.filter((p) => p.sections.issues)
          .map((p) => p.id),
      );
      for (const projectId of Object.keys(get().byProject)) {
        if (shown.has(projectId)) {
          void get().refresh(projectId, { rerun: true, auto: true });
        } else {
          // Stale, so a section switched back on fetches right away.
          patch(projectId, { fetchedAt: 0 });
        }
      }
    },

    refresh: async (projectId, opts) => {
      const drop = drops.get(projectId) ?? 0;
      // Closed since this refresh began: whatever it learned is for nobody.
      const gone = () =>
        (drops.get(projectId) ?? 0) !== drop || !get().byProject[projectId];
      if (opts?.auto) {
        const before = get().byProject[projectId] ?? EMPTY;
        // Already waiting for a slot: that fetch will read the new setups.
        if (autoWaiting.has(projectId)) return;
        // Already fetching: only a setup change needs one more afterwards.
        if (before.loading) {
          if (opts.rerun) rerunAfterFetch.add(projectId);
          return;
        }
        // No setup at all: nothing is fetched, so nothing to pace either.
        const noSetup = get().overview?.setups?.length === 0;
        // A setup change (`rerun`) always paces: after the first setup every
        // project still carries the local "not configured" error, and none of
        // them is free any more.
        if (!noSetup && (opts.rerun || !unpaced(before.error))) {
          // Wait *before* showing it as loading: a queued project is not
          // busy, so a manual refresh can still run (and this one lapses).
          const slot = autoFetchSlot();
          if (slot) {
            const ticket = {};
            autoWaiting.set(projectId, ticket);
            await slot;
            // Lapsed: a manual refresh ran meanwhile, or the project closed.
            if (autoWaiting.get(projectId) !== ticket) return;
            autoWaiting.delete(projectId);
          }
        }
      } else {
        autoWaiting.delete(projectId);
      }
      const current = get().byProject[projectId] ?? EMPTY;
      if (current.loading) {
        // A setup/token change must not wait for the next poll: run once more
        // when the fetch in flight is done.
        if (opts?.rerun) rerunAfterFetch.add(projectId);
        return;
      }
      const { mine } = current;
      patch(projectId, { loading: true });
      if (!get().overview) await get().loadOverview();
      if ((drops.get(projectId) ?? 0) !== drop) return;
      const { overview } = get();
      // The scope was switched mid-fetch: that answer is for the old one, so
      // it must not land on the new scope's board; refetch instead.
      const switched = () =>
        (get().byProject[projectId]?.mine ?? true) !== mine;
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
      } else {
        try {
          const board = await issuesBoard(projectId, mine);
          // Closed meanwhile: do not bring its entry back.
          if (gone()) return;
          patch(
            projectId,
            switched()
              ? { loading: false }
              : { board, error: null, loading: false, fetchedAt: Date.now() },
          );
        } catch (e: unknown) {
          if (gone()) return;
          patch(
            projectId,
            switched()
              ? { loading: false }
              : {
                  error: toIpcError(e),
                  loading: false,
                  fetchedAt: Date.now(),
                },
          );
        }
      }
      const again = rerunAfterFetch.delete(projectId);
      if (gone()) return;
      if (switched()) {
        await get().refresh(projectId);
      } else if (again) {
        await get().refresh(projectId, { auto: true });
      }
    },

    hydrate: async (projectId) => {
      const mine = (get().byProject[projectId] ?? EMPTY).mine;
      const drop = drops.get(projectId) ?? 0;
      try {
        const cached = await issuesBoardCached(projectId, mine);
        // Closed while reading: do not bring its entry back.
        if ((drops.get(projectId) ?? 0) !== drop) return null;
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
      // Stopped (unmounted, project closed) before the cache answered: no
      // first fetch for a board nobody shows any more.
      let stopped = false;
      // A closed project is dropped before its tab unmounts, so `stopped` may
      // still be false when the cache answers: the drop count tells.
      const drop = drops.get(projectId) ?? 0;
      void get()
        .hydrate(projectId)
        .then((at) => {
          if (stopped || (drops.get(projectId) ?? 0) !== drop) return;
          if (at === null || Date.now() - at > ISSUE_POLL_MS) {
            void refresh(projectId, { auto: true });
          }
        });
      const timer = setInterval(
        () => void refresh(projectId, { auto: true }),
        ISSUE_POLL_MS,
      );
      const onFocus = () => {
        const at = get().byProject[projectId]?.fetchedAt ?? 0;
        if (Date.now() - at > ISSUE_FOCUS_STALE_MS) {
          void refresh(projectId, { auto: true });
        }
      };
      window.addEventListener("focus", onFocus);
      return () => {
        stopped = true;
        autoWaiting.delete(projectId);
        clearInterval(timer);
        window.removeEventListener("focus", onFocus);
      };
    },

    dropProject: (projectId) =>
      set((s) => {
        rerunAfterFetch.delete(projectId);
        autoWaiting.delete(projectId);
        drops.set(projectId, (drops.get(projectId) ?? 0) + 1);
        const next = { ...s.byProject };
        delete next[projectId];
        return { byProject: next };
      }),
  };
});
