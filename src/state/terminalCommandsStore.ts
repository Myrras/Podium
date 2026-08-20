/**
 * Per-project terminal quick commands: the user's own commands shown in the
 * Commands modal (Terminals subsection). Keyed by project root path (like
 * layoutStore) so they survive restarts and project reopens; persisted to
 * localStorage — agents don't need these, so no backend store.
 */

import { create } from "zustand";

interface TerminalCommandsPersisted {
  /** User-defined commands per project root, in insertion order. */
  customByRoot: Record<string, string[]>;
}

const STORAGE_KEY = "podium.terminalCommands";

function load(): TerminalCommandsPersisted {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { customByRoot: {} };
    const saved = JSON.parse(raw) as Partial<TerminalCommandsPersisted>;
    return {
      customByRoot:
        saved.customByRoot && typeof saved.customByRoot === "object"
          ? saved.customByRoot
          : {},
    };
  } catch {
    return { customByRoot: {} };
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
}

export const useTerminalCommandsStore = create<TerminalCommandsState>(
  (set, get) => ({
    ...load(),
    addCommand: (root, command) => {
      const trimmed = command.trim();
      if (!trimmed) return;
      const existing = get().customByRoot[root] ?? [];
      if (existing.includes(trimmed)) return;
      const customByRoot = {
        ...get().customByRoot,
        [root]: [...existing, trimmed],
      };
      persist({ customByRoot });
      set({ customByRoot });
    },
    removeCommand: (root, command) => {
      const customByRoot = {
        ...get().customByRoot,
        [root]: (get().customByRoot[root] ?? []).filter((c) => c !== command),
      };
      persist({ customByRoot });
      set({ customByRoot });
    },
  }),
);
