import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  AgentSettingsDto,
  BackendSettingsDto,
  McpClientInfo,
} from "../ipc/types";

// Platform flags, flippable per test: the WSL backend field is Windows-only.
const platform = vi.hoisted(() => ({ isWindows: false }));
vi.mock("../lib/platform", () => ({
  get isWindows() {
    return platform.isWindows;
  },
  isMac: false,
}));

function backend(
  overrides: Partial<BackendSettingsDto> = {},
): BackendSettingsDto {
  return {
    wslDistro: "",
    activeWslDistro: "",
    nativeDriveProjects: false,
    activeNativeDriveProjects: false,
    ...overrides,
  };
}
let backendDto: BackendSettingsDto = backend();

// The settings modal reaches the backend only through the Tauri invoke bridge,
// which jsdom lacks — route every command by name to a canned response.
const sampleIssues = {
  setups: [
    {
      id: "s1",
      name: "Work GitLab",
      provider: "gitlab",
      url: "http://git.example.test",
      tokenSet: true,
    },
  ],
  projects: {},
};

const sampleDto: AgentSettingsDto = {
  mergeMode: "merge",
  defaultAdapter: "",
  suggestWorktree: true,
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

const invoke = vi.fn((cmd: string, args?: unknown) => {
  switch (cmd) {
    case "mcp_clients_status":
      return Promise.resolve(sampleClients);
    case "mcp_client_install":
      return Promise.resolve(sampleClients);
    case "agent_settings_get":
    case "agent_settings_set_adapter":
    case "agent_settings_set_default_adapter":
    case "agent_settings_set_merge_mode":
    case "agent_settings_set_suggest_worktree":
      return Promise.resolve(sampleDto);
    case "issues_settings_get":
    case "issues_setup_save":
    case "issues_setup_remove":
      return Promise.resolve(sampleIssues);
    case "assistant_autonomy_get":
      return Promise.resolve("confirm-destructive");
    case "assistant_autonomy_set":
      return Promise.resolve((args as { mode: string }).mode);
    case "assistant_proactivity_get":
      return Promise.resolve(true);
    case "assistant_proactivity_set":
      return Promise.resolve((args as { enabled: boolean }).enabled);
    case "assistant_polling_get":
      return Promise.resolve({
        intervalMinutes: 10,
        slack: true,
        teams: true,
      });
    case "assistant_polling_set":
      return Promise.resolve(args);
    case "backend_settings_get":
      return Promise.resolve(backendDto);
    case "backend_settings_set_wsl_distro":
      if ((args as { distro: string | null }).distro === "Ubunto") {
        return Promise.reject({
          kind: "invalidInput",
          message: 'WSL distro "Ubunto" is not installed (installed: Ubuntu)',
        });
      }
      backendDto = {
        ...backendDto,
        wslDistro: (args as { distro: string | null }).distro ?? "",
      };
      return Promise.resolve(backendDto);
    case "backend_settings_set_native_drive_projects":
      backendDto = {
        ...backendDto,
        nativeDriveProjects: (args as { enabled: boolean }).enabled,
      };
      return Promise.resolve(backendDto);
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

import { useIssueStore } from "../state/issueStore";
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

  it("persists the suggest-worktree toggle when flipped", async () => {
    await openAgentsTab();
    const toggle = screen.getByRole("switch", {
      name: "Suggest git worktrees",
    });
    expect(toggle).toHaveAttribute("aria-checked", "true"); // default on

    fireEvent.click(toggle);

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "agent_settings_set_suggest_worktree",
        { enabled: false },
      ),
    );
  });
});

describe("SettingsModal — Assistant tab", () => {
  beforeEach(() => {
    invoke.mockClear();
  });

  it("saves the autonomy mode and the proactivity toggle", async () => {
    render(<SettingsModal open onClose={() => undefined} />);
    fireEvent.click(screen.getByRole("tab", { name: "Assistant" }));

    const autonomy = await screen.findByLabelText("Assistant autonomy");
    fireEvent.change(autonomy, { target: { value: "propose-all" } });
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("assistant_autonomy_set", {
        mode: "propose-all",
      }),
    );

    const toggle = screen.getByRole("switch", {
      name: "Let the assistant speak first",
    });
    expect(toggle).toHaveAttribute("aria-checked", "true"); // default on
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("assistant_proactivity_set", {
        enabled: false,
      }),
    );
  });

  it("shows the polling interval and source toggles, and saves changes", async () => {
    render(<SettingsModal open onClose={() => undefined} />);
    fireEvent.click(screen.getByRole("tab", { name: "Assistant" }));

    const interval = await screen.findByLabelText(
      "Sweep interval, in minutes (0 = off)",
    );
    expect(interval).toHaveValue(10);

    fireEvent.change(interval, { target: { value: "30" } });
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("assistant_polling_set", {
        intervalMinutes: 30,
        slack: true,
        teams: true,
      }),
    );

    const slack = screen.getByRole("switch", { name: "Slack" });
    expect(slack).toHaveAttribute("aria-checked", "true"); // default on
    fireEvent.click(slack);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("assistant_polling_set", {
        intervalMinutes: 30,
        slack: false,
        teams: true,
      }),
    );

    const teams = screen.getByRole("switch", { name: "Teams" });
    expect(teams).toHaveAttribute("aria-checked", "true"); // default on
    fireEvent.click(teams);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("assistant_polling_set", {
        intervalMinutes: 30,
        slack: false,
        teams: false,
      }),
    );
  });

  it("resets the conversation from the settings tab", async () => {
    render(<SettingsModal open onClose={() => undefined} />);
    fireEvent.click(screen.getByRole("tab", { name: "Assistant" }));

    fireEvent.click(
      await screen.findByRole("button", { name: "Reset conversation" }),
    );
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "assistant_new_conversation",
        undefined,
      ),
    );
  });
});

describe("SettingsModal — Issues tab", () => {
  beforeEach(() => {
    invoke.mockClear();
    useIssueStore.setState({ overview: null, byProject: {} });
  });

  async function openIssuesTab() {
    render(<SettingsModal open onClose={() => undefined} />);
    fireEvent.click(screen.getByRole("tab", { name: "Issues" }));
    return screen.findByText("Work GitLab");
  }

  it("lists setups without ever rendering a token", async () => {
    await openIssuesTab();
    expect(
      screen.getByText(/http:\/\/git\.example\.test · token saved/),
    ).toBeTruthy();
    expect(document.querySelector('input[type="password"]')).toBeNull();
  });

  it("edits a setup, keeping the token when left blank", async () => {
    await openIssuesTab();
    fireEvent.click(screen.getByText("Edit"));
    const token = screen.getByLabelText("Access token");
    expect(token).toHaveAttribute("type", "password");
    expect(token).toHaveValue("");
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("issues_setup_save", {
        id: "s1",
        name: "Work GitLab",
        provider: "gitlab",
        url: "http://git.example.test",
        account: undefined,
        token: undefined,
      }),
    );
  });

  it("adds a GitHub setup with a gh account", async () => {
    await openIssuesTab();
    fireEvent.click(screen.getByText("Add GitHub setup"));
    expect(screen.getByLabelText("URL")).toHaveValue("https://github.com");
    expect(screen.queryByLabelText("Access token")).toBeNull();
    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "Personal" },
    });
    fireEvent.change(screen.getByLabelText("gh account"), {
      target: { value: "octo-fictional" },
    });
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("issues_setup_save", {
        id: undefined,
        name: "Personal",
        provider: "github",
        url: "https://github.com",
        account: "octo-fictional",
        token: undefined,
      }),
    );
  });

  it("clears a GitLab setup's stored token", async () => {
    await openIssuesTab();
    fireEvent.click(screen.getByText("Edit"));
    fireEvent.click(screen.getByText("Clear token"));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("issues_setup_save", {
        id: "s1",
        name: "Work GitLab",
        provider: "gitlab",
        url: "http://git.example.test",
        token: "",
      }),
    );
  });

  it("removes a setup", async () => {
    await openIssuesTab();
    fireEvent.click(screen.getByLabelText("Remove Work GitLab"));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("issues_setup_remove", { id: "s1" }),
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

describe("SettingsModal — General tab, WSL backend", () => {
  beforeEach(() => {
    invoke.mockClear();
    backendDto = backend();
  });
  afterEach(() => {
    platform.isWindows = false;
  });

  it("fetches the backend settings once for the whole tab", async () => {
    platform.isWindows = true;
    render(<SettingsModal open onClose={() => undefined} />);
    const input = await screen.findByLabelText("WSL distro");
    await waitFor(() => expect(input).not.toBeDisabled());
    expect(
      invoke.mock.calls.filter(([cmd]) => cmd === "backend_settings_get"),
    ).toHaveLength(1);
  });

  it("is hidden off Windows", () => {
    render(<SettingsModal open onClose={() => undefined} />);
    expect(screen.queryByLabelText("WSL distro")).toBeNull();
    expect(invoke).not.toHaveBeenCalledWith("backend_settings_get", undefined);
  });

  it("hints the distro's $SHELL as the terminal default in WSL mode", async () => {
    platform.isWindows = true;
    backendDto = backend({ wslDistro: "Ubuntu", activeWslDistro: "Ubuntu" });
    render(<SettingsModal open onClose={() => undefined} />);
    await waitFor(() =>
      expect(screen.getByLabelText("Shell")).toHaveAttribute(
        "placeholder",
        "$SHELL (default)",
      ),
    );
  });

  it("hints PowerShell for Windows-drive projects when they run natively", async () => {
    platform.isWindows = true;
    backendDto = backend({
      wslDistro: "Ubuntu",
      activeWslDistro: "Ubuntu",
      nativeDriveProjects: true,
      activeNativeDriveProjects: true,
    });
    render(<SettingsModal open onClose={() => undefined} />);
    await waitFor(() =>
      expect(screen.getByLabelText("Shell")).toHaveAttribute(
        "placeholder",
        "$SHELL (default; PowerShell for C:\\ projects)",
      ),
    );
  });

  it("offers the native Windows-drive opt-in only with a distro, off by default", async () => {
    platform.isWindows = true;
    render(<SettingsModal open onClose={() => undefined} />);
    const input = await screen.findByLabelText("WSL distro");
    await waitFor(() => expect(input).not.toBeDisabled());
    const label = "Run Windows-drive projects natively";
    expect(screen.queryByRole("switch", { name: label })).toBeNull();

    fireEvent.change(input, { target: { value: "Ubuntu" } });
    const toggle = screen.getByRole("switch", { name: label });
    expect(toggle).toHaveAttribute("aria-checked", "false");
  });

  it("saves the native Windows-drive opt-in and asks for a restart", async () => {
    platform.isWindows = true;
    backendDto = backend({ wslDistro: "Ubuntu", activeWslDistro: "Ubuntu" });
    render(<SettingsModal open onClose={() => undefined} />);
    const toggle = await screen.findByRole("switch", {
      name: "Run Windows-drive projects natively",
    });
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "backend_settings_set_native_drive_projects",
        { enabled: true },
      ),
    );
    await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "true"));
    expect(
      screen.getByText("Restart Podium to apply this change."),
    ).toBeInTheDocument();
  });

  it("hints PowerShell as the terminal default on native Windows", async () => {
    platform.isWindows = true;
    render(<SettingsModal open onClose={() => undefined} />);
    await screen.findByLabelText("WSL distro");
    expect(screen.getByLabelText("Shell")).toHaveAttribute(
      "placeholder",
      "powershell (default)",
    );
  });

  it("saves the distro on Windows and asks for a restart", async () => {
    platform.isWindows = true;
    render(<SettingsModal open onClose={() => undefined} />);
    const input = await screen.findByLabelText("WSL distro");
    await waitFor(() => expect(input).not.toBeDisabled());
    fireEvent.change(input, { target: { value: " Ubuntu " } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("backend_settings_set_wsl_distro", {
        distro: "Ubuntu",
      }),
    );
    expect(
      await screen.findByText("Restart Podium to apply this change."),
    ).toBeInTheDocument();
  });

  it("shows why a distro that is not installed was refused", async () => {
    platform.isWindows = true;
    render(<SettingsModal open onClose={() => undefined} />);
    const input = await screen.findByLabelText("WSL distro");
    await waitFor(() => expect(input).not.toBeDisabled());
    fireEvent.change(input, { target: { value: "Ubunto" } });
    fireEvent.blur(input);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      'WSL distro "Ubunto" is not installed (installed: Ubuntu)',
    );
    expect(
      screen.queryByText("Restart Podium to apply this change."),
    ).toBeNull();
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
