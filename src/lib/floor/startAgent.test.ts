import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/commands", () => ({}));

import type { ProjectInfo } from "../../ipc/types";
import { useAssistantStore } from "../../state/assistantStore";
import { useFloorStore } from "../../state/floorStore";
import { useProjectStore } from "../../state/projectStore";
import { useSelectionStore } from "../selection";
import { startAgentIn } from "./startAgent";

const project = { id: "proj-1", name: "Webshop" } as ProjectInfo;
const plain = { metaKey: false, ctrlKey: false };

describe("startAgentIn", () => {
  beforeEach(() => {
    useFloorStore.getState().reset();
    useAssistantStore.setState({ enabled: true, draft: null });
    useProjectStore.setState({ activeProjectId: null });
    useSelectionStore.setState({ text: "", items: [] });
  });

  it("adds the project as context once, with no draft, and opens the desk's chat when the assistant is on", () => {
    startAgentIn(project, plain);
    startAgentIn(project, plain);
    expect(useSelectionStore.getState().items).toEqual([
      { kind: "project", projectId: "proj-1", id: "proj-1" },
    ]);
    expect(useAssistantStore.getState().draft).toBeNull();
    expect(useAssistantStore.getState().focusRequest).toBe(2);
    expect(useFloorStore.getState().deskPopover).toBe("chat");
    expect(useFloorStore.getState().agentModal).toBeNull();
    expect(useProjectStore.getState().activeProjectId).toBe("proj-1");
  });

  it.each([
    ["Cmd held", { metaKey: true, ctrlKey: false }, true],
    ["Ctrl held", { metaKey: false, ctrlKey: true }, true],
    ["assistant off", plain, false],
  ])("opens the New agent screen: %s", (_, e, enabled) => {
    useAssistantStore.setState({ enabled });
    startAgentIn(project, e);
    expect(useFloorStore.getState().agentModal).toEqual({
      projectId: "proj-1",
    });
    expect(useAssistantStore.getState().draft).toBeNull();
    expect(useFloorStore.getState().deskPopover).toBeNull();
    expect(useSelectionStore.getState().items).toEqual([]);
    expect(useProjectStore.getState().activeProjectId).toBe("proj-1");
  });
});
