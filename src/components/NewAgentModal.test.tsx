import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The modal talks to the IPC command layer directly (adaptersList /
// agentSettingsGet), so mock it rather than the raw Tauri bridge.
vi.mock("../ipc/commands", () => ({
  adaptersList: vi.fn(),
  agentSettingsGet: vi.fn(),
  toIpcError: (e: unknown) => ({
    kind: "io",
    message: e instanceof Error ? e.message : String(e),
  }),
}));

vi.mock("../state/toastStore", () => ({ toastError: vi.fn() }));

import type {
  AdapterInfo,
  AgentSettingsDto,
  AgentSpawnOptions,
} from "../ipc/types";
import { adaptersList, agentSettingsGet } from "../ipc/commands";
import { useProcessStore } from "../state/processStore";
import { NewAgentModal } from "./NewAgentModal";

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
];

const SETTINGS: AgentSettingsDto = {
  mergeMode: "merge",
  defaultAdapter: "claude-code",
  suggestWorktree: true,
  adapters: [],
};

function seedSpawn() {
  const spawnAgent = vi.fn((_projectId: string, _options: AgentSpawnOptions) =>
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

describe("NewAgentModal worktree checkbox", () => {
  beforeEach(() => {
    useProcessStore.setState(initialProcess, true);
    adaptersListMock.mockResolvedValue(ADAPTERS);
    agentSettingsGetMock.mockResolvedValue(SETTINGS);
  });

  it("passes worktree: true when the checkbox is ticked", async () => {
    const { spawnAgent } = seedSpawn();
    render(
      <NewAgentModal open projectId={PROJECT} onClose={() => undefined} />,
    );
    await screen.findByText("Claude Code");

    fireEvent.click(screen.getByLabelText("Run in a git worktree"));
    fireEvent.click(screen.getByRole("button", { name: "Start agent" }));

    await waitFor(() =>
      expect(spawnAgent).toHaveBeenCalledWith(
        PROJECT,
        expect.objectContaining({ worktree: true }),
      ),
    );
  });

  it("omits worktree when the checkbox is left unticked", async () => {
    const { spawnAgent } = seedSpawn();
    render(
      <NewAgentModal open projectId={PROJECT} onClose={() => undefined} />,
    );
    await screen.findByText("Claude Code");

    fireEvent.click(screen.getByRole("button", { name: "Start agent" }));

    await waitFor(() => expect(spawnAgent).toHaveBeenCalled());
    const options = spawnAgent.mock.calls[0][1];
    expect(options.worktree).toBeUndefined();
  });

  it("seeds the args field from settings and passes edits per session", async () => {
    agentSettingsGetMock.mockResolvedValue({
      ...SETTINGS,
      adapters: [
        {
          id: "claude-code",
          displayName: "Claude Code",
          available: true,
          binary: "claude",
          command: "",
          defaultArgs: ["--model", "opus"],
        },
      ],
    });
    const { spawnAgent } = seedSpawn();
    render(
      <NewAgentModal open projectId={PROJECT} onClose={() => undefined} />,
    );
    await screen.findByText("Claude Code");

    // Seeded from the adapter's default args.
    const argsInput =
      await screen.findByLabelText<HTMLInputElement>("Arguments");
    await waitFor(() => expect(argsInput.value).toBe("--model opus"));

    // Edit for this session and spawn.
    fireEvent.change(argsInput, { target: { value: "--model haiku" } });
    fireEvent.click(screen.getByRole("button", { name: "Start agent" }));

    await waitFor(() =>
      expect(spawnAgent).toHaveBeenCalledWith(
        PROJECT,
        expect.objectContaining({ args: ["--model", "haiku"] }),
      ),
    );
  });
});
