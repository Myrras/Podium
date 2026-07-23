/**
 * Per-project scratchpad lists, keyed by project id.
 *
 * Mutations apply eagerly from the command's return value; the backend's
 * `scratchpad:changed` event (which also fires when an agent edits
 * scratchpads over MCP) triggers a `refresh` that reconciles.
 *
 * `updateContent`/`updateTitle` require the caller's last-seen `updatedAt`
 * (echoed back verbatim — never reparsed/reformatted, since the backend
 * compares it for exact equality). If a concurrent edit landed first, the
 * backend rejects with `IpcError.kind === "scratchpadConflict"`; that case
 * is surfaced to the caller as `{ conflict: true }` instead of a toast,
 * since it needs a user decision (reload vs. force save) rather than a
 * fire-and-forget notification.
 */

import { create } from "zustand";

import {
  scratchpadAdd,
  scratchpadAddTag,
  scratchpadList,
  scratchpadListArchived,
  scratchpadRemove,
  scratchpadRemoveTag,
  scratchpadSetArchived,
  scratchpadUnassign,
  scratchpadUpdateContent,
  scratchpadUpdateTitle,
  toIpcError,
} from "../ipc/commands";
import type { ProjectId, ScratchpadId, ScratchpadInfo } from "../ipc/types";
import { toastError } from "./toastStore";

/** A save that was rejected because someone else edited the scratchpad first. */
export interface ScratchpadConflict {
  conflict: true;
}

/** Returned by conflict-checked mutations: the fresh snapshot, a conflict marker, or `null` on other failure. */
export type ScratchpadUpdateResult = ScratchpadInfo | ScratchpadConflict | null;

function isConflict(e: unknown): boolean {
  return toIpcError(e).kind === "scratchpadConflict";
}

interface ScratchpadState {
  scratchpadsByProject: Record<ProjectId, ScratchpadInfo[]>;
  /** Archived scratchpads per project, loaded on demand (the Archive modal). */
  archivedByProject: Record<ProjectId, ScratchpadInfo[]>;
  /** Re-pull one project's active list (initial load + `scratchpad:changed`). */
  refresh: (projectId: ProjectId) => Promise<void>;
  /** Re-pull one project's archived list (opening the Archive modal). */
  refreshArchived: (projectId: ProjectId) => Promise<void>;
  /** Returns the new scratchpad (or `null` on failure) so callers can open it. */
  addScratchpad: (projectId: ProjectId) => Promise<ScratchpadInfo | null>;
  /** Returns the updated snapshot, a conflict marker, or `null` on other failure. */
  updateContent: (
    projectId: ProjectId,
    id: ScratchpadId,
    content: string,
    expectedUpdatedAt: string,
  ) => Promise<ScratchpadUpdateResult>;
  /** Returns the updated snapshot, a conflict marker, or `null` on other failure. */
  updateTitle: (
    projectId: ProjectId,
    id: ScratchpadId,
    title: string,
    expectedUpdatedAt: string,
  ) => Promise<ScratchpadUpdateResult>;
  /** Add a free-text tag; returns the updated snapshot (or `null`). */
  addTag: (
    projectId: ProjectId,
    id: ScratchpadId,
    tag: string,
  ) => Promise<ScratchpadInfo | null>;
  /** Remove a tag by exact value; returns the updated snapshot (or `null`). */
  removeTag: (
    projectId: ProjectId,
    id: ScratchpadId,
    tag: string,
  ) => Promise<ScratchpadInfo | null>;
  /**
   * Archive or unarchive a scratchpad; updates both the active and archived
   * caches eagerly. Returns the updated snapshot (or `null`).
   */
  setScratchpadArchived: (
    projectId: ProjectId,
    id: ScratchpadId,
    archived: boolean,
  ) => Promise<ScratchpadInfo | null>;
  /**
   * Unassign a scratchpad from its agent (sends a best-effort cancel request
   * to the agent first, server-side). Applies the returned snapshot eagerly;
   * the `scratchpad:changed` refresh reconciles.
   */
  unassignScratchpad: (projectId: ProjectId, id: ScratchpadId) => Promise<void>;
  /** Permanently remove a scratchpad (from the Archive modal). */
  removeScratchpad: (projectId: ProjectId, id: ScratchpadId) => Promise<void>;
  /** Event applier for `project:closed` — drops the cached lists. */
  dropProject: (projectId: ProjectId) => void;
}

export const useScratchpadStore = create<ScratchpadState>((set) => ({
  scratchpadsByProject: {},
  archivedByProject: {},

  refresh: async (projectId) => {
    try {
      const scratchpads = await scratchpadList(projectId);
      set((s) => ({
        scratchpadsByProject: {
          ...s.scratchpadsByProject,
          [projectId]: scratchpads,
        },
      }));
    } catch (e) {
      toastError("Failed to list scratchpads", toIpcError(e).message);
    }
  },

  refreshArchived: async (projectId) => {
    try {
      const scratchpads = await scratchpadListArchived(projectId);
      set((s) => ({
        archivedByProject: {
          ...s.archivedByProject,
          [projectId]: scratchpads,
        },
      }));
    } catch (e) {
      toastError("Failed to list archived scratchpads", toIpcError(e).message);
    }
  },

  addScratchpad: async (projectId) => {
    try {
      const info = await scratchpadAdd(projectId);
      set((s) => {
        const current = s.scratchpadsByProject[projectId] ?? [];
        if (current.some((sp) => sp.id === info.id)) return s;
        return {
          scratchpadsByProject: {
            ...s.scratchpadsByProject,
            [projectId]: [...current, info],
          },
        };
      });
      return info;
    } catch (e) {
      toastError("Could not add scratchpad", toIpcError(e).message);
      return null;
    }
  },

  updateContent: async (projectId, id, content, expectedUpdatedAt) => {
    try {
      const info = await scratchpadUpdateContent(
        projectId,
        id,
        content,
        expectedUpdatedAt,
      );
      set((s) => ({
        scratchpadsByProject: {
          ...s.scratchpadsByProject,
          [projectId]: (s.scratchpadsByProject[projectId] ?? []).map((sp) =>
            sp.id === id ? info : sp,
          ),
        },
      }));
      return info;
    } catch (e) {
      if (isConflict(e)) return { conflict: true };
      toastError("Could not update scratchpad", toIpcError(e).message);
      return null;
    }
  },

  updateTitle: async (projectId, id, title, expectedUpdatedAt) => {
    try {
      const info = await scratchpadUpdateTitle(
        projectId,
        id,
        title,
        expectedUpdatedAt,
      );
      set((s) => ({
        scratchpadsByProject: {
          ...s.scratchpadsByProject,
          [projectId]: (s.scratchpadsByProject[projectId] ?? []).map((sp) =>
            sp.id === id ? info : sp,
          ),
        },
      }));
      return info;
    } catch (e) {
      if (isConflict(e)) return { conflict: true };
      toastError("Could not rename scratchpad", toIpcError(e).message);
      return null;
    }
  },

  addTag: async (projectId, id, tag) => {
    try {
      const info = await scratchpadAddTag(projectId, id, tag);
      set((s) => ({
        scratchpadsByProject: {
          ...s.scratchpadsByProject,
          [projectId]: (s.scratchpadsByProject[projectId] ?? []).map((sp) =>
            sp.id === id ? info : sp,
          ),
        },
      }));
      return info;
    } catch (e) {
      toastError("Could not add tag", toIpcError(e).message);
      return null;
    }
  },

  removeTag: async (projectId, id, tag) => {
    try {
      const info = await scratchpadRemoveTag(projectId, id, tag);
      set((s) => ({
        scratchpadsByProject: {
          ...s.scratchpadsByProject,
          [projectId]: (s.scratchpadsByProject[projectId] ?? []).map((sp) =>
            sp.id === id ? info : sp,
          ),
        },
      }));
      return info;
    } catch (e) {
      toastError("Could not remove tag", toIpcError(e).message);
      return null;
    }
  },

  setScratchpadArchived: async (projectId, id, archived) => {
    try {
      const info = await scratchpadSetArchived(projectId, id, archived);
      set((s) => {
        const active = s.scratchpadsByProject[projectId] ?? [];
        const archivedList = s.archivedByProject[projectId] ?? [];
        return {
          scratchpadsByProject: {
            ...s.scratchpadsByProject,
            [projectId]: archived
              ? active.filter((sp) => sp.id !== id)
              : active.some((sp) => sp.id === id)
                ? active.map((sp) => (sp.id === id ? info : sp))
                : [...active, info],
          },
          archivedByProject: {
            ...s.archivedByProject,
            [projectId]: archived
              ? [info, ...archivedList.filter((sp) => sp.id !== id)]
              : archivedList.filter((sp) => sp.id !== id),
          },
        };
      });
      return info;
    } catch (e) {
      toastError(
        archived
          ? "Could not archive scratchpad"
          : "Could not unarchive scratchpad",
        toIpcError(e).message,
      );
      return null;
    }
  },

  unassignScratchpad: async (projectId, id) => {
    try {
      const info = await scratchpadUnassign(projectId, id);
      set((s) => ({
        scratchpadsByProject: {
          ...s.scratchpadsByProject,
          [projectId]: (s.scratchpadsByProject[projectId] ?? []).map((sp) =>
            sp.id === id ? info : sp,
          ),
        },
      }));
    } catch (e) {
      toastError("Could not unassign scratchpad", toIpcError(e).message);
    }
  },

  removeScratchpad: async (projectId, id) => {
    try {
      await scratchpadRemove(projectId, id);
      // Deletion happens from the Archive modal; drop it from both caches
      // (it may live in either while the modal reconciles).
      set((s) => ({
        scratchpadsByProject: {
          ...s.scratchpadsByProject,
          [projectId]: (s.scratchpadsByProject[projectId] ?? []).filter(
            (sp) => sp.id !== id,
          ),
        },
        archivedByProject: {
          ...s.archivedByProject,
          [projectId]: (s.archivedByProject[projectId] ?? []).filter(
            (sp) => sp.id !== id,
          ),
        },
      }));
    } catch (e) {
      toastError("Could not remove scratchpad", toIpcError(e).message);
    }
  },

  dropProject: (projectId) =>
    set((s) => {
      const next = { ...s.scratchpadsByProject };
      delete next[projectId];
      const nextArchived = { ...s.archivedByProject };
      delete nextArchived[projectId];
      return { scratchpadsByProject: next, archivedByProject: nextArchived };
    }),
}));
