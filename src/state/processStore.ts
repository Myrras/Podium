/**
 * Managed processes and the active (focused) one.
 *
 * Mutations call the backend and rely on the global lifecycle events
 * (`process:added` / `process:removed` / `process:status`) to update the
 * list — those events fire for every source of change (this UI, agents via
 * MCP, project close), so applying them in one place keeps the store from
 * double-handling its own writes. The event ↔ store wiring lives in App.
 */

import { create } from "zustand";

import {
  agentSpawn,
  processAdd,
  processList,
  processRemove,
  processRename,
  processRestart,
  processStart,
  processStop,
  toIpcError,
} from "../ipc/commands";
import type {
  AgentSpawnOptions,
  NewProcess,
  ProcessId,
  ProcessInfo,
  ProcessStatus,
  ProjectId,
} from "../ipc/types";
import { disposeTerminal } from "../lib/terminalRegistry";
import { useLayoutStore } from "./layoutStore";
import { toastError } from "./toastStore";

interface ProcessState {
  processes: ProcessInfo[];
  activeProcessId: ProcessId | null;
  /** Re-pull the full list from the backend (startup / event resync). */
  refresh: () => Promise<void>;
  /** Create a process; returns it (and focuses it) on success. */
  addProcess: (
    projectId: ProjectId,
    spec: NewProcess,
  ) => Promise<ProcessInfo | null>;
  /** Spawn (add + start) an agent; returns it (and focuses it) on success. */
  spawnAgent: (
    projectId: ProjectId,
    options: AgentSpawnOptions,
  ) => Promise<ProcessInfo | null>;
  removeProcess: (id: ProcessId) => Promise<void>;
  /** Rename a process's display label; blank names are rejected. */
  renameProcess: (id: ProcessId, name: string) => Promise<void>;
  startProcess: (id: ProcessId) => Promise<void>;
  stopProcess: (id: ProcessId) => Promise<void>;
  restartProcess: (id: ProcessId) => Promise<void>;
  setActiveProcess: (id: ProcessId | null) => void;
  /** Event applier for `process:status`. */
  applyStatus: (processId: ProcessId, status: ProcessStatus) => void;
  /** Event applier for `process:removed` — also disposes the terminal. */
  applyRemoved: (processId: ProcessId) => void;
}

export const useProcessStore = create<ProcessState>((set) => ({
  processes: [],
  activeProcessId: null,

  refresh: async () => {
    try {
      const processes = await processList();
      set((s) => ({
        processes,
        activeProcessId: processes.some((p) => p.id === s.activeProcessId)
          ? s.activeProcessId
          : null,
      }));
    } catch (e) {
      toastError("Failed to list processes", toIpcError(e).message);
    }
  },

  addProcess: async (projectId, spec) => {
    try {
      const info = await processAdd(projectId, spec);
      // Insert eagerly so the caller can focus it; the `process:added`
      // refresh reconciles (insertion is idempotent by id).
      set((s) => ({
        processes: s.processes.some((p) => p.id === info.id)
          ? s.processes
          : [...s.processes, info],
        activeProcessId: info.id,
      }));
      return info;
    } catch (e) {
      toastError("Could not create process", toIpcError(e).message);
      return null;
    }
  },

  spawnAgent: async (projectId, options) => {
    try {
      const info = await agentSpawn(projectId, options);
      // Insert eagerly so the caller can focus it; the `process:added`
      // refresh reconciles (insertion is idempotent by id).
      set((s) => ({
        processes: s.processes.some((p) => p.id === info.id)
          ? s.processes
          : [...s.processes, info],
        activeProcessId: info.id,
      }));
      return info;
    } catch (e) {
      toastError("Could not start agent", toIpcError(e).message);
      return null;
    }
  },

  removeProcess: async (id) => {
    try {
      await processRemove(id);
    } catch (e) {
      toastError("Could not remove process", toIpcError(e).message);
    }
  },

  renameProcess: async (id, name) => {
    try {
      const info = await processRename(id, name);
      // Apply eagerly; the `process:updated` refresh reconciles by id.
      set((s) => ({
        processes: s.processes.map((p) => (p.id === id ? info : p)),
      }));
    } catch (e) {
      toastError("Could not rename process", toIpcError(e).message);
    }
  },

  startProcess: async (id) => {
    try {
      await processStart(id);
    } catch (e) {
      toastError("Could not start process", toIpcError(e).message);
    }
  },

  stopProcess: async (id) => {
    try {
      await processStop(id);
    } catch (e) {
      toastError("Could not stop process", toIpcError(e).message);
    }
  },

  restartProcess: async (id) => {
    try {
      await processRestart(id);
    } catch (e) {
      toastError("Could not restart process", toIpcError(e).message);
    }
  },

  setActiveProcess: (id) => {
    // Focusing a process takes over the work area from any open to-do or
    // scratchpad.
    if (id !== null) {
      useLayoutStore.getState().clearOpenTodo();
      useLayoutStore.getState().clearOpenScratchpad();
    }
    set({ activeProcessId: id });
  },

  applyStatus: (processId, status) =>
    set((s) => ({
      processes: s.processes.map((p) =>
        p.id === processId ? { ...p, status } : p,
      ),
    })),

  applyRemoved: (processId) => {
    disposeTerminal(processId);
    set((s) => ({
      processes: s.processes.filter((p) => p.id !== processId),
      activeProcessId:
        s.activeProcessId === processId ? null : s.activeProcessId,
    }));
  },
}));
