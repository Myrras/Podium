import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/commands", () => ({}));

import type { ProjectInfo } from "../../ipc/types";
import { useAssistantStore } from "../../state/assistantStore";
import { useFloorStore } from "../../state/floorStore";
import { useProjectStore } from "../../state/projectStore";
import { startAgentIn } from "./startAgent";

const project = { id: "proj-1", name: "Webshop" } as ProjectInfo;
const plain = { metaKey: false, ctrlKey: false };

describe("startAgentIn", () => {
  beforeEach(() => {
    useFloorStore.getState().reset();
    useAssistantStore.setState({ enabled: true, draft: null });
    useProjectStore.setState({ activeProjectId: null });
  });

  it("drafts a prompt in the desk's chat and opens it when the assistant is on", () => {
    startAgentIn(project, plain);
    expect(useAssistantStore.getState().draft).toBe(
      "Start an agent in Webshop on ",
    );
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
    expect(useProjectStore.getState().activeProjectId).toBe("proj-1");
  });
});
