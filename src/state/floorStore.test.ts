import { beforeEach, describe, expect, it, vi } from "vitest";

import { useAssistantStore } from "./assistantStore";
import { useFloorStore } from "./floorStore";

const popover = { projectId: "proj-1", kind: "todos", itemId: null } as const;

beforeEach(() => useFloorStore.getState().reset());

describe("floorStore", () => {
  it("openTerminal tells the assistant the agent was opened", () => {
    const spy = vi.fn();
    useAssistantStore.setState({ handleAgentOpened: spy });
    useFloorStore.getState().openTerminal("proc-1");
    expect(spy).toHaveBeenCalledWith("proc-1");
    expect(useFloorStore.getState().terminalId).toBe("proc-1");
  });

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

  it("keeps one popover across the fixtures and the front desk", () => {
    const s = useFloorStore.getState();
    s.openPopover(popover);
    s.openDesk("chat");
    expect(useFloorStore.getState()).toMatchObject({
      popover: null,
      deskPopover: "chat",
    });
    s.openDesk("list");
    expect(useFloorStore.getState().deskPopover).toBe("list");
    s.openPopover(popover);
    expect(useFloorStore.getState().deskPopover).toBeNull();
    s.openDesk("chat");
    s.openTerminal("proc-1");
    expect(useFloorStore.getState().deskPopover).toBeNull();
    s.openDesk("list");
    s.closeDesk();
    expect(useFloorStore.getState().deskPopover).toBeNull();
  });

  it("toggles a desk popover", () => {
    const s = useFloorStore.getState();
    s.openPopover(popover);
    s.toggleDesk("chat");
    expect(useFloorStore.getState()).toMatchObject({
      popover: null,
      deskPopover: "chat",
    });
    s.toggleDesk("list");
    expect(useFloorStore.getState().deskPopover).toBe("list");
    s.toggleDesk("list");
    expect(useFloorStore.getState().deskPopover).toBeNull();
  });

  it("reset drops everything", () => {
    const s = useFloorStore.getState();
    s.setViewport({ w: 800, h: 600 });
    s.setHoverDesk("proc-1");
    useFloorStore.setState({ fitted: true });
    s.setCamera({ x: 1, y: 2, z: 1 });
    expect(useFloorStore.getState().moved).toBe(true);
    s.setAgentModal({ projectId: "proj-1" });
    s.setScratchpadAgentModal({ projectId: "proj-1", scratchpadIds: ["s-1"] });
    s.reset();
    expect(useFloorStore.getState()).toMatchObject({
      viewport: { w: 0, h: 0 },
      hoverDesk: null,
      agentModal: null,
      scratchpadAgentModal: null,
      fitted: false,
      moved: false,
    });
  });
});
