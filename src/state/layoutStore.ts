/** Persistent sidebar layout: width and per-project collapse state. */

import { create } from "zustand";

import type { ProjectId, TodoId } from "../ipc/types";
import { useProcessStore } from "./processStore";

interface LayoutPersisted {
  sidebarWidth: number;
  /** Collapsed project groups, keyed by project root path (default expanded). */
  collapsedProjects: Record<string, boolean>;
}

const STORAGE_KEY = "podium.layout";

const DEFAULTS: LayoutPersisted = {
  sidebarWidth: 280,
  collapsedProjects: {},
};

function load(): LayoutPersisted {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return structuredClone(DEFAULTS);
    // Older payloads stored per-section collapse; anything missing falls back.
    const saved = JSON.parse(raw) as Partial<LayoutPersisted>;
    return {
      sidebarWidth:
        typeof saved.sidebarWidth === "number"
          ? saved.sidebarWidth
          : DEFAULTS.sidebarWidth,
      collapsedProjects:
        saved.collapsedProjects && typeof saved.collapsedProjects === "object"
          ? saved.collapsedProjects
          : {},
    };
  } catch {
    return structuredClone(DEFAULTS);
  }
}

function persist(state: LayoutPersisted) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    /* quota exceeded */
  }
}

/** The to-do currently filling the work area (opened from the sidebar). */
export interface OpenTodo {
  projectId: ProjectId;
  todoId: TodoId;
}

export interface LayoutState extends LayoutPersisted {
  /**
   * To-do shown in the work area, or `null` when a process/welcome screen is
   * shown instead. In-memory (not persisted) — the work area starts empty.
   * Mutually exclusive with the focused process: opening one clears the other.
   */
  openTodo: OpenTodo | null;
  setSidebarWidth: (w: number) => void;
  toggleProjectCollapsed: (root: string) => void;
  /** Fill the work area with a to-do (clears the focused process). */
  openTodoInWorkArea: (projectId: ProjectId, todoId: TodoId) => void;
  /** Clear the open to-do (e.g. when a process takes the work area). */
  clearOpenTodo: () => void;
}

export const useLayoutStore = create<LayoutState>((set, get) => {
  const initial = load();
  return {
    ...initial,
    openTodo: null,
    setSidebarWidth: (w) => {
      const sidebarWidth = Math.min(600, Math.max(160, w));
      const s = get();
      persist({ sidebarWidth, collapsedProjects: s.collapsedProjects });
      set({ sidebarWidth });
    },
    toggleProjectCollapsed: (root) => {
      const s = get();
      const collapsedProjects = {
        ...s.collapsedProjects,
        [root]: !s.collapsedProjects[root],
      };
      persist({ sidebarWidth: s.sidebarWidth, collapsedProjects });
      set({ collapsedProjects });
    },
    openTodoInWorkArea: (projectId, todoId) => {
      // A to-do and a process are mutually exclusive in the work area.
      useProcessStore.getState().setActiveProcess(null);
      set({ openTodo: { projectId, todoId } });
    },
    clearOpenTodo: () => set({ openTodo: null }),
  };
});
