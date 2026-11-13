import { beforeEach, describe, expect, it } from "vitest";

import { useFloorStore } from "./floorStore";

const popover = { projectId: "proj-1", kind: "todos", itemId: null } as const;

beforeEach(() => useFloorStore.getState().reset());

describe("floorStore", () => {
  it("starts at the defaults", () => {
    const s = useFloorStore.getState();
    expect(s.camera).toEqual({ x: 0, y: 0, z: 1 });
    expect(s.viewport).toEqual({ w: 0, h: 0 });
    expect(s.focus).toBeNull();
    expect(s.animate).toBe(false);
    expect(s.popover).toBeNull();
    expect(s.terminalId).toBeNull();
  });

  it("setCamera defaults focus to null and animate to false", () => {
    const s = useFloorStore.getState();
    s.setCamera({ x: 10, y: 20, z: 0.5 }, { focus: "all", animate: true });
    expect(useFloorStore.getState()).toMatchObject({
      camera: { x: 10, y: 20, z: 0.5 },
      focus: "all",
      animate: true,
    });
    s.setCamera({ x: 1, y: 2, z: 1 });
    expect(useFloorStore.getState()).toMatchObject({
      focus: null,
      animate: false,
    });
  });

  it("keeps one popover and sets its item", () => {
    const s = useFloorStore.getState();
    s.setPopoverItem("todo-1"); // no popover: nothing happens
    expect(useFloorStore.getState().popover).toBeNull();
    s.openPopover(popover);
    s.openPopover({ ...popover, kind: "menu" });
    s.setPopoverItem("todo-1");
    expect(useFloorStore.getState().popover).toEqual({
      projectId: "proj-1",
      kind: "menu",
      itemId: "todo-1",
    });
    s.closePopover();
    expect(useFloorStore.getState().popover).toBeNull();
  });

  it("opening a terminal closes the popover", () => {
    const s = useFloorStore.getState();
    s.openPopover(popover);
    s.openTerminal("proc-1");
    expect(useFloorStore.getState()).toMatchObject({
      terminalId: "proc-1",
      popover: null,
    });
    s.closeTerminal();
    expect(useFloorStore.getState().terminalId).toBeNull();
  });

  it("reset drops everything", () => {
    const s = useFloorStore.getState();
    s.setViewport({ w: 800, h: 600 });
    s.setHoverDesk("proc-1");
    s.setAgentModal({ projectId: "proj-1" });
    s.setScratchpadAgentModal({ projectId: "proj-1", scratchpadIds: ["s-1"] });
    s.reset();
    expect(useFloorStore.getState()).toMatchObject({
      viewport: { w: 0, h: 0 },
      hoverDesk: null,
      agentModal: null,
      scratchpadAgentModal: null,
    });
  });
});
