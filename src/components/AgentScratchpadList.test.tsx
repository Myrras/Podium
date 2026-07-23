import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The stores pull in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

import type { AssignedAgent, ScratchpadInfo } from "../ipc/types";
import { useLayoutStore } from "../state/layoutStore";
import { useProjectStore } from "../state/projectStore";
import { useScratchpadStore } from "../state/scratchpadStore";
import { AgentScratchpadList } from "./AgentScratchpadList";

const initialScratchpad = useScratchpadStore.getState();
const initialLayout = useLayoutStore.getState();
const initialProject = useProjectStore.getState();

const PROJECT = "proj-1";
const AGENT = "agent-1";

function scratchpad(
  id: string,
  title: string,
  assignedAgent:
    (Omit<AssignedAgent, "color"> & { color?: string | null }) | null,
): ScratchpadInfo {
  return {
    id,
    projectId: PROJECT,
    title,
    content: "",
    archived: false,
    archivedAt: null,
    createdAt: "2024-04-03T12:00:00Z",
    updatedAt: "2024-04-03T12:00:00Z",
    updatedBy: "User",
    version: 1,
    tags: [],
    assignedAgent: assignedAgent ? { color: null, ...assignedAgent } : null,
  };
}

function seed(scratchpads: ScratchpadInfo[]) {
  const unassignScratchpad = vi.fn(() => Promise.resolve());
  useScratchpadStore.setState(
    {
      ...initialScratchpad,
      scratchpadsByProject: { [PROJECT]: scratchpads },
      unassignScratchpad,
    },
    true,
  );
  const setActiveProject = vi.fn();
  useProjectStore.setState({ ...initialProject, setActiveProject }, true);
  const openScratchpadInWorkArea = vi.fn();
  useLayoutStore.setState({ ...initialLayout, openScratchpadInWorkArea }, true);
  return { unassignScratchpad, setActiveProject, openScratchpadInWorkArea };
}

describe("AgentScratchpadList", () => {
  beforeEach(() => {
    useScratchpadStore.setState(initialScratchpad, true);
    useProjectStore.setState(initialProject, true);
    useLayoutStore.setState(initialLayout, true);
  });

  it("lists only the scratchpads assigned to this agent", () => {
    seed([
      scratchpad("a", "Design notes", { processId: AGENT, name: "claude" }),
      scratchpad("b", "Other notes", { processId: "other", name: "beta" }),
      scratchpad("c", "Unassigned", null),
    ]);
    render(<AgentScratchpadList projectId={PROJECT} processId={AGENT} />);

    expect(screen.getByText("Design notes")).toBeInTheDocument();
    expect(screen.queryByText("Other notes")).not.toBeInTheDocument();
    expect(screen.queryByText("Unassigned")).not.toBeInTheDocument();
  });

  it("renders nothing when the agent has no assigned scratchpads", () => {
    seed([
      scratchpad("b", "Other notes", { processId: "other", name: "beta" }),
    ]);
    const { container } = render(
      <AgentScratchpadList projectId={PROJECT} processId={AGENT} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("unassigns the scratchpad when the (x) is clicked", () => {
    const { unassignScratchpad } = seed([
      scratchpad("a", "Design notes", { processId: AGENT, name: "claude" }),
    ]);
    render(<AgentScratchpadList projectId={PROJECT} processId={AGENT} />);

    fireEvent.click(
      screen.getByRole("button", { name: /Stop this agent and unassign/ }),
    );

    expect(unassignScratchpad).toHaveBeenCalledWith(PROJECT, "a");
  });

  it("navigates to the scratchpad when the title is clicked", () => {
    const { setActiveProject, openScratchpadInWorkArea, unassignScratchpad } =
      seed([
        scratchpad("a", "Design notes", { processId: AGENT, name: "claude" }),
      ]);
    render(<AgentScratchpadList projectId={PROJECT} processId={AGENT} />);

    fireEvent.click(screen.getByText("Design notes"));

    expect(setActiveProject).toHaveBeenCalledWith(PROJECT);
    expect(openScratchpadInWorkArea).toHaveBeenCalledWith(PROJECT, "a");
    expect(unassignScratchpad).not.toHaveBeenCalled();
  });

  it("clicking the (x) does not also trigger navigation", () => {
    const { openScratchpadInWorkArea } = seed([
      scratchpad("a", "Design notes", { processId: AGENT, name: "claude" }),
    ]);
    render(<AgentScratchpadList projectId={PROJECT} processId={AGENT} />);

    fireEvent.click(
      screen.getByRole("button", { name: /Stop this agent and unassign/ }),
    );

    expect(openScratchpadInWorkArea).not.toHaveBeenCalled();
  });
});
