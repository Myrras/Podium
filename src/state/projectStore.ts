/** Workspace projects, the last-interacted selection, and recents — synced with the backend. */

import { create } from "zustand";
import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";

import {
  projectClose,
  projectConfigReload,
  projectList,
  projectOpen,
  projectRename,
  projectReorder,
  recentsList,
  recentsRemove,
  toIpcError,
  workspaceList,
  workspaceRemove,
} from "../ipc/commands";
import type { ProjectId, ProjectInfo, RecentProject } from "../ipc/types";
import { toastError, useToastStore } from "./toastStore";

/**
 * Move the project with id `id` to sit before the project with id `beforeId`
 * (or to the end when `beforeId` is null), returning the reordered ids. Pure
 * so it can be unit-tested and drive an optimistic UI update.
 */
export function reorderIds(
  ids: ProjectId[],
  id: ProjectId,
  beforeId: ProjectId | null,
): ProjectId[] {
  const without = ids.filter((x) => x !== id);
  if (beforeId === null || beforeId === id) return [...without, id];
  const at = without.indexOf(beforeId);
  if (at === -1) return [...without, id];
  return [...without.slice(0, at), id, ...without.slice(at)];
}

interface ProjectState {
  projects: ProjectInfo[];
  /** Last-interacted project — default target for modals; the sidebar shows all. */
  activeProjectId: ProjectId | null;
  /** Recently opened projects, most recent first. */
  recents: RecentProject[];
  /** Re-pull the full list from the backend (startup / event resync). */
  refresh: () => Promise<void>;
  /** Re-pull the recents list (startup / after opening a project). */
  refreshRecents: () => Promise<void>;
  /**
   * Re-open every persisted workspace project (startup). An entry that fails
   * to open is kept (not pruned — a transient failure, e.g. an unmounted
   * drive, must not silently drop the project) and its error toast offers a
   * "Remove from workspace" action for permanent failures.
   */
  restoreWorkspace: () => Promise<void>;
  /** Open the folder at `path` as a project; activates it on success. */
  openProject: (path: string) => Promise<void>;
  /** Native folder picker → `openProject`. */
  openProjectDialog: () => Promise<void>;
  /** Remove the project from the sidebar; the backend stops its processes
   *  and drops it from the persisted workspace. */
  closeProject: (id: ProjectId) => Promise<void>;
  /** Re-read `podium.yml` (config errors surface on the project). */
  reloadProjectConfig: (id: ProjectId) => Promise<void>;
  /** Rename a project; a blank/null name reverts to the folder/config name. */
  renameProject: (id: ProjectId, name: string | null) => Promise<void>;
  /**
   * Move project `id` before `beforeId` (or to the end when null),
   * optimistically updating the list and persisting the new order.
   */
  reorderProjects: (id: ProjectId, beforeId: ProjectId | null) => Promise<void>;
  removeRecent: (path: string) => Promise<void>;
  setActiveProject: (id: ProjectId | null) => void;
}

export const useProjectStore = create<ProjectState>((set, get) => ({
  projects: [],
  activeProjectId: null,
  recents: [],

  refresh: async () => {
    try {
      const projects = await projectList();
      set((s) => ({
        projects,
        activeProjectId: projects.some((p) => p.id === s.activeProjectId)
          ? s.activeProjectId
          : (projects[0]?.id ?? null),
      }));
    } catch (e) {
      toastError("Failed to list projects", toIpcError(e).message);
    }
  },

  refreshRecents: async () => {
    try {
      set({ recents: await recentsList() });
    } catch (e) {
      toastError("Failed to load recent projects", toIpcError(e).message);
    }
  },

  restoreWorkspace: async () => {
    let paths: string[] = [];
    try {
      paths = await workspaceList();
    } catch (e) {
      toastError("Failed to load workspace", toIpcError(e).message);
    }
    // Open sequentially so the sidebar keeps the persisted workspace order.
    // A failed open is kept in the workspace: once a project is added it stays
    // added until the user explicitly closes it. Transient failures (e.g. an
    // external/network drive not yet mounted at startup) must not silently drop
    // the project — it comes back on the next launch when the folder is there.
    // For a permanent failure (folder moved/deleted), the toast's action lets
    // the user drop the entry instead of seeing it fail on every launch.
    for (const path of paths) {
      try {
        await projectOpen(path);
      } catch (e) {
        let toastId = -1;
        toastId = toastError(`Could not restore ${path}`, toIpcError(e).message, {
          sticky: true,
          action: {
            label: "Remove from workspace",
            onClick: async () => {
              try {
                await workspaceRemove(path);
                useToastStore.getState().requestDismiss(toastId);
              } catch (removeErr) {
                toastError(
                  "Could not remove from workspace",
                  toIpcError(removeErr).message,
                );
              }
            },
          },
        });
      }
    }
    await get().refresh();
    await get().refreshRecents();
  },

  openProject: async (path) => {
    try {
      const project = await projectOpen(path);
      set((s) => ({
        projects: s.projects.some((p) => p.id === project.id)
          ? s.projects
          : [...s.projects, project],
        activeProjectId: project.id,
      }));
    } catch (e) {
      toastError("Could not open project", toIpcError(e).message);
      return;
    }
    // The backend pushed this project onto the recents list; re-pull it.
    await get().refreshRecents();
  },

  openProjectDialog: async () => {
    let path: string | null;
    try {
      path = await openFolderDialog({
        directory: true,
        multiple: false,
        title: "Add project folder",
      });
    } catch (e) {
      toastError("Folder picker failed", toIpcError(e).message);
      return;
    }
    if (!path) return; // user cancelled
    await get().openProject(path);
  },

  closeProject: async (id) => {
    try {
      await projectClose(id);
    } catch (e) {
      toastError("Could not close project", toIpcError(e).message);
      return;
    }
    set((s) => {
      const projects = s.projects.filter((p) => p.id !== id);
      return {
        projects,
        activeProjectId:
          s.activeProjectId === id
            ? (projects[0]?.id ?? null)
            : s.activeProjectId,
      };
    });
  },

  reloadProjectConfig: async (id) => {
    try {
      const project = await projectConfigReload(id);
      set((s) => ({
        projects: s.projects.map((p) => (p.id === project.id ? project : p)),
      }));
    } catch (e) {
      toastError("Could not reload project config", toIpcError(e).message);
    }
  },

  renameProject: async (id, name) => {
    try {
      const project = await projectRename(id, name);
      set((s) => ({
        projects: s.projects.map((p) => (p.id === project.id ? project : p)),
      }));
    } catch (e) {
      toastError("Could not rename project", toIpcError(e).message);
    }
  },

  reorderProjects: async (id, beforeId) => {
    const prev = get().projects;
    const order = reorderIds(
      prev.map((p) => p.id),
      id,
      beforeId,
    );
    // Optimistic reorder; roll back to the backend's answer (or the previous
    // list) if persisting fails.
    const byId = new Map(prev.map((p) => [p.id, p]));
    set({
      projects: order.map((pid) => byId.get(pid)!).filter(Boolean),
    });
    try {
      set({ projects: await projectReorder(order) });
    } catch (e) {
      set({ projects: prev });
      toastError("Could not reorder projects", toIpcError(e).message);
    }
  },

  removeRecent: async (path) => {
    try {
      set({ recents: await recentsRemove(path) });
    } catch (e) {
      toastError("Could not remove recent project", toIpcError(e).message);
    }
  },

  setActiveProject: (id) => set({ activeProjectId: id }),
}));
