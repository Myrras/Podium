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

/**
 * `sections` of `root` collapsed or expanded. Expanding deletes the key rather
 * than storing `false` (expanded is the default), so the persisted map only
 * ever holds what is actually collapsed.
 */
function withSections(
  current: Record<string, boolean>,
  root: string,
  sections: readonly Section[],
  collapsed: boolean,
): Record<string, boolean> {
  const next = { ...current };
  for (const section of sections) {
    const key = sectionKey(root, section);
    if (collapsed) next[key] = true;
    else delete next[key];
  }
  return next;
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

/** The project whose Git board fills the work area (opened from its Git tab). */
export interface OpenBoard {
  projectId: ProjectId;
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
  /**
   * Git board shown in the work area, or `null`. In-memory. Mutually
   * exclusive with the focused process, to-do and scratchpad.
   */
  openBoard: OpenBoard | null;
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
  /**
   * Drop a project's persisted collapse state (its group and subsections)
   * once it leaves the sidebar, so `podium.layout` does not keep growing.
   */
  forgetProject: (root: string) => void;
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
  /** Fill the work area with a project's Git board (clears the focused process/to-do/scratchpad). */
  openBoardInWorkArea: (projectId: ProjectId) => void;
  /** Clear the open board (e.g. when a process takes the work area). */
  clearOpenBoard: () => void;
}

export const useLayoutStore = create<LayoutState>((set, get) => {
  const initial = load();
  return {
    ...initial,
    openTodo: null,
    openScratchpad: null,
    openBoard: null,
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
      const collapsedSections = withSections(
        s.collapsedSections,
        root,
        [section],
        collapsed,
      );
      persist({ ...persistable(s), collapsedSections });
      set({ collapsedSections });
    },
    setAllSectionsCollapsed: (root, collapsed) => {
      const s = get();
      const collapsedSections = withSections(
        s.collapsedSections,
        root,
        SECTIONS,
        collapsed,
      );
      persist({ ...persistable(s), collapsedSections });
      set({ collapsedSections });
    },
    forgetProject: (root) => {
      const s = get();
      const collapsedSections = withSections(
        s.collapsedSections,
        root,
        SECTIONS,
        false,
      );
      const collapsedProjects = { ...s.collapsedProjects };
      delete collapsedProjects[root];
      persist({ ...persistable(s), collapsedSections, collapsedProjects });
      set({ collapsedSections, collapsedProjects });
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
      set({
        openTodo: { projectId, todoId },
        openScratchpad: null,
        openBoard: null,
      });
    },
    clearOpenTodo: () => set({ openTodo: null }),
    openScratchpadInWorkArea: (projectId, scratchpadId) => {
      useProcessStore.getState().setActiveProcess(null);
      set({
        openScratchpad: { projectId, scratchpadId },
        openTodo: null,
        openBoard: null,
      });
    },
    clearOpenScratchpad: () => set({ openScratchpad: null }),
    openBoardInWorkArea: (projectId) => {
      useProcessStore.getState().setActiveProcess(null);
      set({
        openBoard: { projectId },
        openTodo: null,
        openScratchpad: null,
      });
    },
    clearOpenBoard: () => set({ openBoard: null }),
  };
});
