/**
 * One-shot "open Settings on this tab" request, so a component deep in the
 * tree (e.g. an inline "check Settings" link) can reach the modal `App` owns.
 */

import { create } from "zustand";

export type SettingsTabId =
  "general" | "agents" | "assistant" | "issues" | "mcp";

interface SettingsRequestState {
  request: SettingsTabId | null;
  open: (tab: SettingsTabId) => void;
  clear: () => void;
}

export const useSettingsRequestStore = create<SettingsRequestState>((set) => ({
  request: null,
  open: (tab) => set({ request: tab }),
  clear: () => set({ request: null }),
}));
