/**
 * The changes panel's state: which checkout it shows (a project root or one
 * of its worktrees), against which base, the changed-file list and the
 * diffs of the files the user expanded.
 *
 * Nothing here is pushed by the backend — git is the source of truth, so the
 * panel pulls: on open, on a target/base switch, on window focus, on a slow
 * poll and on the refresh button. Each pull carries a request number, and an
 * answer that is not the latest is dropped, so a slow `git diff` for the
 * previous target can never overwrite the current one.
 */

import { create } from "zustand";

import { changesFileDiff, changesList, toIpcError } from "../ipc/commands";
import type {
  ChangeSet,
  ChangedFile,
  ChangesBase,
  FileDiff,
  ProjectId,
} from "../ipc/types";

/** A checkout: the project root (`worktree` null) or one of its worktrees. */
export interface ChangesTarget {
  projectId: ProjectId;
  worktree: string | null;
}

export function targetKey(t: ChangesTarget): string {
  return `${t.projectId}:${t.worktree ?? ""}`;
}

export interface DiffEntry {
  loading: boolean;
  diff: FileDiff | null;
  error: string | null;
}

interface ChangesState {
  /** A checkout the user picked; `null` follows what is focused. */
  pinned: ChangesTarget | null;
  base: ChangesBase;
  /** `targetKey` + base the current `changes` belong to. */
  loadedKey: string | null;
  changes: ChangeSet | null;
  loading: boolean;
  error: string | null;
  /** Expanded files, by path. */
  expanded: Record<string, boolean>;
  /** Diffs of expanded files, by path. */
  diffs: Record<string, DiffEntry>;
  pin: (target: ChangesTarget | null) => void;
  setBase: (base: ChangesBase) => void;
  /** Pull the file list for `target` (and re-pull the expanded diffs). */
  load: (target: ChangesTarget) => Promise<void>;
  /** Expand or collapse a file; expanding fetches its diff. */
  toggleFile: (target: ChangesTarget, file: ChangedFile) => void;
  /** Expand or collapse every listed file at once. */
  setAllExpanded: (target: ChangesTarget, expanded: boolean) => void;
}

let listRequest = 0;
const diffRequests: Record<string, number> = {};

function errorText(e: unknown): string {
  const err = toIpcError(e);
  if (err.kind === "notAGitRepo") return "Not a git repository.";
  if (err.kind === "worktreeNotFound") return "That worktree no longer exists.";
  return err.message;
}

function loadKey(target: ChangesTarget, base: ChangesBase): string {
  return `${targetKey(target)}|${base}`;
}

export const useChangesStore = create<ChangesState>((set, get) => {
  async function fetchDiff(target: ChangesTarget, file: ChangedFile) {
    const key = loadKey(target, get().base);
    const token = (diffRequests[file.path] ?? 0) + 1;
    diffRequests[file.path] = token;
    set((s) => ({
      diffs: {
        ...s.diffs,
        [file.path]: {
          diff: s.diffs[file.path]?.diff ?? null,
          error: null,
          loading: true,
        },
      },
    }));
    let entry: DiffEntry;
    try {
      const diff = await changesFileDiff(
        target.projectId,
        target.worktree,
        get().base,
        file.path,
        file.oldPath,
      );
      entry = { loading: false, diff, error: null };
    } catch (e) {
      entry = { loading: false, diff: null, error: errorText(e) };
    }
    // A newer request for this file, or a switch of target/base, wins.
    if (diffRequests[file.path] !== token || get().loadedKey !== key) return;
    set((s) => ({ diffs: { ...s.diffs, [file.path]: entry } }));
  }

  return {
    pinned: null,
    base: "uncommitted",
    loadedKey: null,
    changes: null,
    loading: false,
    error: null,
    expanded: {},
    diffs: {},

    pin: (pinned) => set({ pinned }),

    setBase: (base) => set({ base }),

    load: async (target) => {
      const key = loadKey(target, get().base);
      const switched = get().loadedKey !== key;
      const token = ++listRequest;
      set(
        switched
          ? {
              loadedKey: key,
              changes: null,
              error: null,
              loading: true,
              expanded: {},
              diffs: {},
            }
          : { loading: true },
      );
      let changes: ChangeSet;
      try {
        changes = await changesList(
          target.projectId,
          target.worktree,
          get().base,
        );
      } catch (e) {
        if (token !== listRequest) return;
        set({ loading: false, error: errorText(e), changes: null });
        return;
      }
      if (token !== listRequest) return;
      // Keep only the expanded files that are still changed, and refresh
      // their diffs: the content may have moved on since they were opened.
      const byPath = new Map(changes.files.map((f) => [f.path, f]));
      const expanded: Record<string, boolean> = {};
      for (const path of Object.keys(get().expanded)) {
        if (get().expanded[path] && byPath.has(path)) expanded[path] = true;
      }
      const diffs: Record<string, DiffEntry> = {};
      for (const path of Object.keys(expanded)) {
        const old = get().diffs[path];
        if (old) diffs[path] = old;
      }
      set({ changes, loading: false, error: null, expanded, diffs });
      for (const path of Object.keys(expanded)) {
        void fetchDiff(target, byPath.get(path)!);
      }
    },

    toggleFile: (target, file) => {
      const open = !get().expanded[file.path];
      set((s) => ({ expanded: { ...s.expanded, [file.path]: open } }));
      if (open && !get().diffs[file.path]?.diff) void fetchDiff(target, file);
    },

    setAllExpanded: (target, open) => {
      const files = get().changes?.files ?? [];
      if (!open) {
        set({ expanded: {} });
        return;
      }
      set({ expanded: Object.fromEntries(files.map((f) => [f.path, true])) });
      for (const f of files) {
        if (!get().diffs[f.path]?.diff) void fetchDiff(target, f);
      }
    },
  };
});
