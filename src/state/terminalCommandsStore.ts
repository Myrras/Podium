/**
 * Per-project terminal quick commands: the user's own commands plus the
 * collapse state of the "Commands" block in the Terminals subsection. Keyed
 * by project root path (like layoutStore) so they survive restarts and
 * project reopens; persisted to localStorage — agents don't need these, so
 * no backend store.
 */

import { create } from "zustand";

interface TerminalCommandsPersisted {
  /** User-defined commands per project root, in insertion order. */
  customByRoot: Record<string, string[]>;
  /** Commands-block collapse per project root (default collapsed). */
  expandedByRoot: Record<string, boolean>;
}

const STORAGE_KEY = "podium.terminalCommands";

const DEFAULTS: TerminalCommandsPersisted = {
  customByRoot: {},
  expandedByRoot: {},
};

function load(): TerminalCommandsPersisted {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return structuredClone(DEFAULTS);
    const saved = JSON.parse(raw) as Partial<TerminalCommandsPersisted>;
    return {
      customByRoot:
        saved.customByRoot && typeof saved.customByRoot === "object"
          ? saved.customByRoot
          : {},
      expandedByRoot:
        saved.expandedByRoot && typeof saved.expandedByRoot === "object"
          ? saved.expandedByRoot
          : {},
    };
  } catch {
    return structuredClone(DEFAULTS);
  }
}

function persist(state: TerminalCommandsPersisted) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    /* quota exceeded */
  }
}

interface TerminalCommandsState extends TerminalCommandsPersisted {
  addCommand: (root: string, command: string) => void;
  removeCommand: (root: string, command: string) => void;
  toggleExpanded: (root: string) => void;
}

export const useTerminalCommandsStore = create<TerminalCommandsState>(
  (set, get) => ({
    ...load(),
    addCommand: (root, command) => {
      const trimmed = command.trim();
      if (!trimmed) return;
      const s = get();
      const existing = s.customByRoot[root] ?? [];
      if (existing.includes(trimmed)) return;
      const customByRoot = {
        ...s.customByRoot,
        [root]: [...existing, trimmed],
      };
      persist({ customByRoot, expandedByRoot: s.expandedByRoot });
      set({ customByRoot });
    },
    removeCommand: (root, command) => {
      const s = get();
      const customByRoot = {
        ...s.customByRoot,
        [root]: (s.customByRoot[root] ?? []).filter((c) => c !== command),
      };
      persist({ customByRoot, expandedByRoot: s.expandedByRoot });
      set({ customByRoot });
    },
    toggleExpanded: (root) => {
      const s = get();
      const expandedByRoot = {
        ...s.expandedByRoot,
        [root]: !s.expandedByRoot[root],
      };
      persist({ customByRoot: s.customByRoot, expandedByRoot });
      set({ expandedByRoot });
    },
  }),
);
