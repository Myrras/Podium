import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentSettingsDto } from "../ipc/types";

// The settings modal reaches the backend only through the Tauri invoke bridge,
// which jsdom lacks — route every command by name to a canned response.
const sampleDto: AgentSettingsDto = {
  mergeMode: "merge",
  defaultAdapter: "",
  adapters: [
    {
      id: "claude-code",
      displayName: "Claude Code",
      available: true,
      binary: "claude",
      command: "",
      defaultArgs: [],
    },
    {
      id: "auggie",
      displayName: "Auggie",
      available: true,
      binary: "auggie",
      command: "",
      defaultArgs: [],
    },
  ],
};

const invoke = vi.fn((cmd: string, _args?: unknown) => {
  switch (cmd) {
    case "mcp_clients_status":
      return Promise.resolve([]);
    case "agent_settings_get":
    case "agent_settings_set_adapter":
    case "agent_settings_set_default_adapter":
    case "agent_settings_set_merge_mode":
      return Promise.resolve(sampleDto);
    default:
      return Promise.resolve(null);
  }
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invoke(cmd, args),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

import { SettingsModal } from "./SettingsModal";

async function openAgentsTab() {
  render(<SettingsModal open onClose={() => undefined} />);
  fireEvent.click(screen.getByRole("tab", { name: "Agents" }));
  // Wait for the tab to finish loading (unique heading; "Claude Code" now
  // appears both as a card and as a Default-agent option).
  return screen.findByText("Argument merge");
}

describe("SettingsModal — Agents tab", () => {
  beforeEach(() => {
    invoke.mockClear();
  });

  it("shows an adapter card with its default command", async () => {
    await openAgentsTab();
    expect(invoke).toHaveBeenCalledWith("agent_settings_get", undefined);
    // With no override, the card previews the built-in binary.
    expect(screen.getByText("claude")).toBeInTheDocument();
  });

  it("saves an override with parsed default arguments", async () => {
    await openAgentsTab();
    // Two adapter cards now; edit the first (Claude Code).
    fireEvent.click(screen.getAllByRole("button", { name: "Edit" })[0]);

    fireEvent.change(screen.getByLabelText("Default arguments"), {
      target: { value: "  --model   opus  " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("agent_settings_set_adapter", {
        adapterId: "claude-code",
        command: null,
        defaultArgs: ["--model", "opus"],
      }),
    );
  });

  it("persists the default agent when changed", async () => {
    await openAgentsTab();
    fireEvent.change(screen.getByLabelText("Default agent adapter"), {
      target: { value: "auggie" },
    });

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "agent_settings_set_default_adapter",
        { adapterId: "auggie" },
      ),
    );
  });

  it("persists the merge mode when changed", async () => {
    await openAgentsTab();
    fireEvent.change(screen.getByLabelText("Argument merge mode"), {
      target: { value: "project-overrides" },
    });

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("agent_settings_set_merge_mode", {
        mode: "project-overrides",
      }),
    );
  });
});
