import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Stand-ins that show the props they got; the real modals have their own tests.
vi.mock("../NewAgentModal", () => ({
  NewAgentModal: (p: {
    open: boolean;
    projectId: string | null;
    todoIds?: string[];
    onClose: () => void;
  }) =>
    p.open ? (
      <button type="button" onClick={p.onClose}>
        agent {p.projectId} {p.todoIds?.join(",")}
      </button>
    ) : null,
}));
vi.mock("../ScratchpadAgentModal", () => ({
  ScratchpadAgentModal: (p: {
    open: boolean;
    projectId: string | null;
    scratchpadIds: string[];
    onClose: () => void;
  }) =>
    p.open ? (
      <button type="button" onClick={p.onClose}>
        scratchpad {p.projectId} {p.scratchpadIds.join(",")}
      </button>
    ) : null,
}));

import { useFloorStore } from "../../state/floorStore";
import { FloorAgentModals } from "./FloorAgentModals";

describe("FloorAgentModals", () => {
  beforeEach(() => useFloorStore.getState().reset());

  it("renders nothing while no modal is requested", () => {
    render(<FloorAgentModals />);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("opens the New agent screen from agentModal and clears it on close", async () => {
    useFloorStore.setState({
      agentModal: { projectId: "proj-1", todoIds: ["todo-1"] },
    });
    render(<FloorAgentModals />);
    await userEvent.click(
      screen.getByRole("button", { name: "agent proj-1 todo-1" }),
    );
    expect(useFloorStore.getState().agentModal).toBeNull();
  });

  it("opens the scratchpad agent screen from scratchpadAgentModal and clears it on close", async () => {
    useFloorStore.setState({
      scratchpadAgentModal: { projectId: "proj-1", scratchpadIds: ["pad-1"] },
    });
    render(<FloorAgentModals />);
    await userEvent.click(
      screen.getByRole("button", { name: "scratchpad proj-1 pad-1" }),
    );
    expect(useFloorStore.getState().scratchpadAgentModal).toBeNull();
  });
});
