/** Persistent sidebar layout: width and per-project collapse state. */

import { create } from "zustand";

import type { ProjectId, ScratchpadId, TodoId } from "../ipc/types";
import { useProcessStore } from "./processStore";

interface LayoutPersisted {
  sidebarWidth: number;
  /** Collapsed project groups, keyed by project root path (default expanded). */
  collapsedProjects: Record<string, boolean>;
  /**
   * Collapsed subsections (To-dos, Agents, …), keyed by `sectionKey(root,
   * section)` (default expanded).
   */
  collapsedSections: Record<string, boolean>;
  /** Whether the assistant chat panel is expanded on the right. */
  assistantOpen: boolean;
  /** Width of the assistant chat panel in px. */
  assistantWidth: number;
}

const STORAGE_KEY = "podium.layout";

const DEFAULTS: LayoutPersisted = {
  sidebarWidth: 280,
  collapsedProjects: {},
  collapsedSections: {},
  assistantOpen: false,
  assistantWidth: 340,
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
      collapsedSections:
        saved.collapsedSections && typeof saved.collapsedSections === "object"
          ? saved.collapsedSections
          : {},
      assistantOpen: saved.assistantOpen === true,
      assistantWidth:
        typeof saved.assistantWidth === "number"
          ? saved.assistantWidth
          : DEFAULTS.assistantWidth,
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

/** The persisted subset of the layout state, for writing it back. */
function persistable(state: LayoutPersisted): LayoutPersisted {
  return {
    sidebarWidth: state.sidebarWidth,
    collapsedProjects: state.collapsedProjects,
    collapsedSections: state.collapsedSections,
    assistantOpen: state.assistantOpen,
    assistantWidth: state.assistantWidth,
  };
}

/** The collapsible subsections of a project group, in sidebar order. */
export const SECTIONS = [
  "todos",
  "agents",
  "processes",
  "terminals",
  "worktrees",
  "scratchpads",
] as const;
export type Section = (typeof SECTIONS)[number];

/** Key into `collapsedSections` for one project's subsection. */
export function sectionKey(root: string, section: Section): string {
  return `${root}\n${section}`;
}

/** The to-do currently filling the work area (opened from the sidebar). */
export interface OpenTodo {
  projectId: ProjectId;
  todoId: TodoId;
}

/** The scratchpad currently filling the work area (opened from the sidebar). */
export interface OpenScratchpad {
  projectId: ProjectId;
  scratchpadId: ScratchpadId;
}

export interface LayoutState extends LayoutPersisted {
  /**
   * To-do shown in the work area, or `null` when a process/scratchpad/welcome
   * screen is shown instead. In-memory (not persisted) — the work area starts
   * empty. Mutually exclusive with the focused process and the open
   * scratchpad: opening one clears the others.
   */
  openTodo: OpenTodo | null;
  /**
   * Scratchpad shown in the work area, or `null` when a process/to-do/welcome
   * screen is shown instead. In-memory (not persisted). Mutually exclusive
   * with the focused process and the open to-do.
   */
  openScratchpad: OpenScratchpad | null;
  setSidebarWidth: (w: number) => void;
  toggleProjectCollapsed: (root: string) => void;
  /** Collapse or expand one subsection of a project (persisted). */
  setSectionCollapsed: (
    root: string,
    section: Section,
    collapsed: boolean,
  ) => void;
  /** Collapse or expand every subsection of a project at once (persisted). */
  setAllSectionsCollapsed: (root: string, collapsed: boolean) => void;
  /** Show or hide the assistant chat panel (persisted). */
  setAssistantOpen: (open: boolean) => void;
  /** Resize the assistant chat panel (persisted, clamped). */
  setAssistantWidth: (w: number) => void;
  /** Fill the work area with a to-do (clears the focused process/scratchpad). */
  openTodoInWorkArea: (projectId: ProjectId, todoId: TodoId) => void;
  /** Clear the open to-do (e.g. when a process takes the work area). */
  clearOpenTodo: () => void;
  /** Fill the work area with a scratchpad (clears the focused process/to-do). */
  openScratchpadInWorkArea: (
    projectId: ProjectId,
    scratchpadId: ScratchpadId,
  ) => void;
  /** Clear the open scratchpad (e.g. when a process takes the work area). */
  clearOpenScratchpad: () => void;
}

export const useLayoutStore = create<LayoutState>((set, get) => {
  const initial = load();
  return {
    ...initial,
    openTodo: null,
    openScratchpad: null,
    setSidebarWidth: (w) => {
      const sidebarWidth = Math.min(600, Math.max(160, w));
      const s = get();
      persist({ ...persistable(s), sidebarWidth });
      set({ sidebarWidth });
    },
    toggleProjectCollapsed: (root) => {
      const s = get();
      const collapsedProjects = {
        ...s.collapsedProjects,
        [root]: !s.collapsedProjects[root],
      };
      persist({ ...persistable(s), collapsedProjects });
      set({ collapsedProjects });
    },
    setSectionCollapsed: (root, section, collapsed) => {
      const s = get();
      const collapsedSections = {
        ...s.collapsedSections,
        [sectionKey(root, section)]: collapsed,
      };
      persist({ ...persistable(s), collapsedSections });
      set({ collapsedSections });
    },
    setAllSectionsCollapsed: (root, collapsed) => {
      const s = get();
      const collapsedSections = { ...s.collapsedSections };
      for (const section of SECTIONS) {
        collapsedSections[sectionKey(root, section)] = collapsed;
      }
      persist({ ...persistable(s), collapsedSections });
      set({ collapsedSections });
    },
    setAssistantOpen: (assistantOpen) => {
      persist({ ...persistable(get()), assistantOpen });
      set({ assistantOpen });
    },
    setAssistantWidth: (w) => {
      const assistantWidth = Math.min(640, Math.max(280, w));
      persist({ ...persistable(get()), assistantWidth });
      set({ assistantWidth });
    },
    openTodoInWorkArea: (projectId, todoId) => {
      // A to-do, a scratchpad, and a process are mutually exclusive in the
      // work area.
      useProcessStore.getState().setActiveProcess(null);
      set({ openTodo: { projectId, todoId }, openScratchpad: null });
    },
    clearOpenTodo: () => set({ openTodo: null }),
    openScratchpadInWorkArea: (projectId, scratchpadId) => {
      useProcessStore.getState().setActiveProcess(null);
      set({ openScratchpad: { projectId, scratchpadId }, openTodo: null });
    },
    clearOpenScratchpad: () => set({ openScratchpad: null }),
  };
});
