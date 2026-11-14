/** In-memory state of the Floor: camera, viewport, open popover, modal and hover. */

import { create } from "zustand";

import type { ProcessId, ProjectId } from "../ipc/types";
import type {
  AgentModalRequest,
  Camera,
  FloorPopover,
  ScratchpadAgentModalRequest,
  Size,
} from "../lib/floor/types";

interface FloorData {
  camera: Camera;
  /** The last camera change should transition. */
  animate: boolean;
  viewport: Size;
  /** What the camera frames; `null` after a manual pan or zoom. */
  focus: "all" | ProjectId | null;
  popover: FloorPopover | null;
  /** The process whose terminal is open in the modal. */
  terminalId: ProcessId | null;
  hoverDesk: ProcessId | null;
  agentModal: AgentModalRequest | null;
  scratchpadAgentModal: ScratchpadAgentModalRequest | null;
  /** FloorMap fitted all once; survives its remount (the Git board), so pan and zoom stay. */
  fitted: boolean;
}

export interface FloorState extends FloorData {
  setCamera: (
    camera: Camera,
    opts?: { focus?: "all" | ProjectId | null; animate?: boolean },
  ) => void;
  setViewport: (size: Size) => void;
  /** Open a popover, replacing the one that is open. */
  openPopover: (popover: FloorPopover) => void;
  setPopoverItem: (itemId: string | null) => void;
  closePopover: () => void;
  /** Open a terminal in the modal; also closes the popover. */
  openTerminal: (id: ProcessId) => void;
  closeTerminal: () => void;
  setHoverDesk: (id: ProcessId | null) => void;
  setAgentModal: (request: AgentModalRequest | null) => void;
  setScratchpadAgentModal: (
    request: ScratchpadAgentModalRequest | null,
  ) => void;
  /** Back to the defaults; Floor calls it on unmount. */
  reset: () => void;
}

const initial = (): FloorData => ({
  camera: { x: 0, y: 0, z: 1 },
  animate: false,
  viewport: { w: 0, h: 0 },
  focus: null,
  popover: null,
  terminalId: null,
  hoverDesk: null,
  agentModal: null,
  scratchpadAgentModal: null,
  fitted: false,
});

export const useFloorStore = create<FloorState>((set) => ({
  ...initial(),
  setCamera: (camera, opts) =>
    set({
      camera,
      focus: opts?.focus ?? null,
      animate: opts?.animate ?? false,
    }),
  setViewport: (viewport) => set({ viewport }),
  openPopover: (popover) => set({ popover }),
  setPopoverItem: (itemId) =>
    set((s) => (s.popover ? { popover: { ...s.popover, itemId } } : {})),
  closePopover: () => set({ popover: null }),
  openTerminal: (terminalId) => set({ terminalId, popover: null }),
  closeTerminal: () => set({ terminalId: null }),
  setHoverDesk: (hoverDesk) => set({ hoverDesk }),
  setAgentModal: (agentModal) => set({ agentModal }),
  setScratchpadAgentModal: (scratchpadAgentModal) =>
    set({ scratchpadAgentModal }),
  reset: () => set(initial()),
}));
