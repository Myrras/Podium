import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The modal talks to the IPC command layer directly (adaptersList /
// agentSettingsGet), so mock it rather than the raw Tauri bridge.
vi.mock("../ipc/commands", () => ({
  adaptersList: vi.fn(),
  agentSettingsGet: vi.fn(),
  toIpcError: (e: unknown) => {
    if (typeof e === "object" && e !== null && "kind" in e && "message" in e) {
      return e as { kind: string; message: string };
    }
    return { kind: "io", message: e instanceof Error ? e.message : String(e) };
  },
}));

vi.mock("../state/toastStore", () => ({ toastError: vi.fn() }));

import type { AdapterInfo, AgentSettingsDto } from "../ipc/types";
import { adaptersList, agentSettingsGet } from "../ipc/commands";
import { useProcessStore } from "../state/processStore";
import { ScratchpadAgentModal } from "./ScratchpadAgentModal";

const adaptersListMock = vi.mocked(adaptersList);
const agentSettingsGetMock = vi.mocked(agentSettingsGet);

const initialProcess = useProcessStore.getState();

const PROJECT = "proj-1";

const ADAPTERS: AdapterInfo[] = [
  {
    id: "claude-code",
    displayName: "Claude Code",
    binary: "claude",
    available: true,
  },
  { id: "auggie", displayName: "Auggie", binary: "auggie", available: false },
];

const SETTINGS: AgentSettingsDto = {
  mergeMode: "merge",
  defaultAdapter: "claude-code",
  suggestWorktree: true,
  adapters: [],
};

function seedSpawn() {
  const spawnAgent = vi.fn(() =>
    Promise.resolve({
      id: "proc-1",
      projectId: PROJECT,
      name: "claude",
      kind: { kind: "agent" as const, adapter: "claude-code" },
      status: {
        state: "running" as const,
        pid: 1,
        since: "2026-07-14T00:00:00Z",
      },
      restartPolicy: "never" as const,
      command: "claude",
      worktree: null,
      color: null,
    }),
  );
  useProcessStore.setState({ ...initialProcess, spawnAgent }, true);
  return { spawnAgent };
}

describe("ScratchpadAgentModal", () => {
  beforeEach(() => {
    useProcessStore.setState(initialProcess, true);
    adaptersListMock.mockResolvedValue(ADAPTERS);
    agentSettingsGetMock.mockResolvedValue(SETTINGS);
  });

  it("renders adapter options once loaded", async () => {
    seedSpawn();
    render(
      <ScratchpadAgentModal
        open
        projectId={PROJECT}
        scratchpadIds={["sp-1"]}
        onClose={vi.fn()}
      />,
    );

    await waitFor(() =>
      expect(screen.getByText("Claude Code")).toBeInTheDocument(),
    );
    expect(screen.getByText(/Auggie \(not installed\)/)).toBeInTheDocument();
  });

  it("shows the singular title for one scratchpad and plural for several", async () => {
    seedSpawn();
    const { rerender } = render(
      <ScratchpadAgentModal
        open
        projectId={PROJECT}
        scratchpadIds={["sp-1"]}
        onClose={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText("Claude Code")).toBeInTheDocument(),
    );
    expect(screen.getByText("Start agent on scratchpad")).toBeInTheDocument();

    rerender(
      <ScratchpadAgentModal
        open
        projectId={PROJECT}
        scratchpadIds={["sp-1", "sp-2"]}
        onClose={vi.fn()}
      />,
    );
    expect(
      screen.getByText("Start agent on 2 scratchpads"),
    ).toBeInTheDocument();
  });

  it("disables Start until an available adapter is selected", async () => {
    seedSpawn();
    adaptersListMock.mockResolvedValue([
      {
        id: "auggie",
        displayName: "Auggie",
        binary: "auggie",
        available: false,
      },
    ]);
    render(
      <ScratchpadAgentModal
        open
        projectId={PROJECT}
        scratchpadIds={["sp-1"]}
        onClose={vi.fn()}
      />,
    );

    await waitFor(() =>
      expect(screen.getByText("Auggie (not installed)")).toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: "Start agent" })).toBeDisabled();
  });

  it("calls spawnAgent with the right args and closes on success", async () => {
    const { spawnAgent } = seedSpawn();
    const onClose = vi.fn();
    render(
      <ScratchpadAgentModal
        open
        projectId={PROJECT}
        scratchpadIds={["sp-1", "sp-2"]}
        initialName="My scratchpad"
        onClose={onClose}
      />,
    );

    await waitFor(() =>
      expect(screen.getByText("Claude Code")).toBeInTheDocument(),
    );

    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "  Custom name  " },
    });
    fireEvent.change(
      screen.getByLabelText("Additional instructions (optional)"),
      { target: { value: "  Please review this  " } },
    );

    fireEvent.click(screen.getByRole("button", { name: "Start agent" }));

    await waitFor(() => expect(spawnAgent).toHaveBeenCalledTimes(1));
    expect(spawnAgent).toHaveBeenCalledWith(PROJECT, {
      adapterId: "claude-code",
      name: "Custom name",
      prompt: "Please review this",
      scratchpadIds: ["sp-1", "sp-2"],
    });
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("prefills the name field from initialName", async () => {
    seedSpawn();
    render(
      <ScratchpadAgentModal
        open
        projectId={PROJECT}
        scratchpadIds={["sp-1"]}
        initialName="07-14 Scratchpad"
        onClose={vi.fn()}
      />,
    );

    await waitFor(() =>
      expect(screen.getByLabelText("Name")).toHaveValue("07-14 Scratchpad"),
    );
  });
});
