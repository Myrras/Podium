/**
 * Per-project terminal quick commands: the user's own commands shown in the
 * Commands modal (Terminals subsection). Keyed by project root path (like
 * layoutStore) so they survive restarts and project reopens; persisted to
 * localStorage — agents don't need these, so no backend store.
 *
 * Each entry carries an optional display name; a terminal launched from it is
 * named after that instead of the raw command line.
 */

import { create } from "zustand";

export interface TerminalCommand {
  command: string;
  /** Display name; the launched terminal inherits it. Blank = use `command`. */
  name?: string;
}

interface TerminalCommandsPersisted {
  /** User-defined commands per project root, in insertion order. */
  customByRoot: Record<string, TerminalCommand[]>;
}

const STORAGE_KEY = "podium.terminalCommands";

/** Pre-naming versions stored bare command strings. */
function migrate(entries: unknown): TerminalCommand[] {
  if (!Array.isArray(entries)) return [];
  return entries
    .map((e) =>
      typeof e === "string"
        ? { command: e }
        : e && typeof (e as TerminalCommand).command === "string"
          ? (e as TerminalCommand)
          : null,
    )
    .filter((e): e is TerminalCommand => e !== null);
}

function load(): TerminalCommandsPersisted {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { customByRoot: {} };
    const saved = JSON.parse(raw) as Partial<TerminalCommandsPersisted>;
    const customByRoot: Record<string, TerminalCommand[]> = {};
    for (const [root, entries] of Object.entries(saved.customByRoot ?? {})) {
      customByRoot[root] = migrate(entries);
    }
    return { customByRoot };
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
  addCommand: (root: string, command: string, name?: string) => void;
  /** Replace the entry at `index`; a blank command is ignored. */
  updateCommand: (root: string, index: number, entry: TerminalCommand) => void;
  removeCommand: (root: string, index: number) => void;
}

/** Trim, dropping an empty name so it never persists as `""`. */
function clean(entry: TerminalCommand): TerminalCommand | null {
  const command = entry.command.trim();
  if (!command) return null;
  const name = entry.name?.trim();
  return name ? { command, name } : { command };
}

export const useTerminalCommandsStore = create<TerminalCommandsState>(
  (set, get) => {
    const write = (root: string, entries: TerminalCommand[]) => {
      const customByRoot = { ...get().customByRoot, [root]: entries };
      persist({ customByRoot });
      set({ customByRoot });
    };
    return {
      ...load(),
      addCommand: (root, command, name) => {
        const entry = clean({ command, name });
        if (!entry) return;
        const existing = get().customByRoot[root] ?? [];
        if (existing.some((e) => e.command === entry.command)) return;
        write(root, [...existing, entry]);
      },
      updateCommand: (root, index, entry) => {
        const cleaned = clean(entry);
        const existing = get().customByRoot[root] ?? [];
        if (!cleaned || !existing[index]) return;
        write(
          root,
          existing.map((e, i) => (i === index ? cleaned : e)),
        );
      },
      removeCommand: (root, index) => {
        write(
          root,
          (get().customByRoot[root] ?? []).filter((_, i) => i !== index),
        );
      },
    };
  },
);
