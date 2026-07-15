import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentSettingsDto, McpClientInfo } from "../ipc/types";

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

const sampleClients: McpClientInfo[] = [
  {
    id: "claude-code",
    displayName: "Claude Code",
    cliAvailable: true,
    installed: true,
    installCommand:
      "claude mcp add --scope user --transport stdio podium -- /App/Podium mcp-bridge",
    checkCommand: "claude mcp list",
  },
  {
    id: "auggie",
    displayName: "Auggie",
    cliAvailable: true,
    installed: false,
    installCommand:
      "auggie mcp add podium --command /App/Podium --args mcp-bridge --replace",
    checkCommand: "auggie mcp list",
  },
];

const invoke = vi.fn((cmd: string, _args?: unknown) => {
  switch (cmd) {
    case "mcp_clients_status":
      return Promise.resolve(sampleClients);
    case "mcp_client_install":
      return Promise.resolve(sampleClients);
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

// SettingsModal transitively imports the notify helper; keep its Tauri plugins
// inert under jsdom.
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: () => Promise.resolve(false),
  requestPermission: () => Promise.resolve("denied"),
  sendNotification: () => undefined,
}));
vi.mock("@tauri-apps/plugin-log", () => ({
  error: () => Promise.resolve(),
  warn: () => Promise.resolve(),
}));

import { SettingsModal } from "./SettingsModal";
import { useSettingsStore } from "../state/settingsStore";

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

describe("SettingsModal — General tab", () => {
  it("persists the preferred terminal shell", () => {
    render(<SettingsModal open onClose={() => undefined} />);
    fireEvent.change(screen.getByLabelText("Shell"), {
      target: { value: "pwsh" },
    });
    expect(useSettingsStore.getState().terminal.shell).toBe("pwsh");
  });
});

describe("SettingsModal — MCP tab", () => {
  beforeEach(() => {
    invoke.mockClear();
  });

  it("lists both external clients with per-client check commands", async () => {
    render(<SettingsModal open onClose={() => undefined} />);
    fireEvent.click(screen.getByRole("tab", { name: "MCP" }));

    expect(await screen.findByText("Claude Code")).toBeInTheDocument();
    expect(screen.getByText("Auggie")).toBeInTheDocument();
    // The hint shows each client's own list command.
    expect(screen.getByText("claude mcp list")).toBeInTheDocument();
    expect(screen.getByText("auggie mcp list")).toBeInTheDocument();
  });

  it("registers the Auggie bridge when its Run is pressed", async () => {
    render(<SettingsModal open onClose={() => undefined} />);
    fireEvent.click(screen.getByRole("tab", { name: "MCP" }));
    await screen.findByText("Auggie");

    // Cards render in list order; the second Run button is Auggie's.
    fireEvent.click(screen.getAllByRole("button", { name: "Run" })[1]);

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("mcp_client_install", {
        clientId: "auggie",
      }),
    );
  });
});

describe("SettingsModal — Notifications", () => {
  beforeEach(() => localStorage.clear());

  it("toggles the sound setting and persists it", () => {
    render(<SettingsModal open onClose={() => undefined} />);
    const toggle = screen.getByRole("switch", { name: "Play sound" });
    expect(toggle).toHaveAttribute("aria-checked", "true"); // default on

    fireEvent.click(toggle);

    expect(toggle).toHaveAttribute("aria-checked", "false");
    const saved = JSON.parse(localStorage.getItem("podium.settings") ?? "{}");
    expect(saved.notifications.sound).toBe(false);
  });
});
