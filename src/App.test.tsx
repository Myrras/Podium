import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// jsdom leaves `navigator.platform` empty, so pin the platform explicitly;
// each test picks one via `platform`. Getters, so a change is seen at render.
const platform = vi.hoisted(() => ({ mac: false, windows: false }));
vi.mock("./lib/platform", () => ({
  get isMac() {
    return platform.mac;
  },
  get isWindows() {
    return platform.windows;
  },
}));
// What `backend_wsl_networking_mode` answers (`null` = native backend).
const backend = vi.hoisted(() => ({ networkingMode: null as string | null }));
// What `assistant_subagents_list` answers.
const assistant = vi.hoisted(() => ({
  subagents: [] as unknown[],
  setResult: true,
}));

// App wires up IPC on mount; jsdom has no Tauri bridge, so stub it out.
// `invoke` resolves per-command fixtures (all list commands — including
// `workspace_list` — return arrays) and `listen` resolves a no-op unlisten.
const invoke = vi.hoisted(() =>
  vi.fn((cmd: string) =>
    Promise.resolve(
      cmd === "adapters_list"
        ? [{ id: "claude-code", displayName: "Claude Code", available: true }]
        : cmd === "backend_wsl_networking_mode"
          ? backend.networkingMode
          : cmd === "assistant_subagents_list"
            ? assistant.subagents
            : cmd === "assistant_enabled_get"
              ? true
              : cmd === "assistant_enabled_set"
                ? assistant.setResult
                : [],
    ),
  ),
);
vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
import { listen } from "@tauri-apps/api/event";
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve(null)),
}));
// xterm needs a real canvas; an opened sub-agent's pane header is enough.
vi.mock("./components/TerminalView", () => ({
  TerminalView: () => <div data-testid="terminal-view" />,
}));
// WindowControls (Windows only) talks to the window API on mount.
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isMaximized: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => undefined),
    minimize: () => Promise.resolve(),
    toggleMaximize: () => Promise.resolve(),
    close: () => Promise.resolve(),
  }),
}));

import userEvent from "@testing-library/user-event";

import { useAssistantStore } from "./state/assistantStore";
import { useLayoutStore } from "./state/layoutStore";
import { useToastStore } from "./state/toastStore";
import App from "./App";

beforeEach(() => {
  platform.mac = false;
  platform.windows = false;
  backend.networkingMode = null;
  assistant.subagents = [];
  assistant.setResult = true;
  useAssistantStore.setState({ subagents: [], enabled: true });
  useLayoutStore.setState({ openSubagent: null });
  invoke.mockClear();
  useToastStore.setState({ toasts: [] });
});

describe("App — WSL networking warning", () => {
  const warning = "WSL networking is not mirrored";

  it("warns once, stickily, when WSL networking is not mirrored", async () => {
    platform.windows = true;
    backend.networkingMode = "nat";
    render(<App />);

    expect(await screen.findByText(warning)).toBeInTheDocument();
    const toasts = useToastStore.getState().toasts;
    expect(toasts.filter((t) => t.message === warning)).toHaveLength(1);
    expect(toasts.find((t) => t.message === warning)?.sticky).toBe(true);
  });

  it.each([["mirrored"], [null]])(
    "stays quiet when the mode is %s",
    async (mode) => {
      platform.windows = true;
      backend.networkingMode = mode;
      render(<App />);

      await waitFor(() =>
        expect(invoke).toHaveBeenCalledWith("backend_wsl_networking_mode"),
      );
      expect(screen.queryByText(warning)).toBeNull();
    },
  );

  it("does not ask off Windows", async () => {
    backend.networkingMode = "nat";
    render(<App />);

    await screen.findByText("Welcome to Podium");
    expect(invoke).not.toHaveBeenCalledWith("backend_wsl_networking_mode");
    expect(screen.queryByText(warning)).toBeNull();
  });
});

describe("App", () => {
  it("renders the shell with the brand and the empty sidebar state", () => {
    render(<App />);

    expect(screen.getByText("Podium")).toBeInTheDocument();
    expect(screen.getByText("Welcome to Podium")).toBeInTheDocument();
    expect(
      screen.getByText("No projects yet. Add one to get started."),
    ).toBeInTheDocument();
  });

  it("shows the add-project affordances while the workspace is empty", () => {
    render(<App />);

    expect(screen.getByText("Add Project…")).toBeInTheDocument();
    expect(screen.getByText("Add project…")).toBeInTheDocument();
  });

  it("toggles the assistant panel from the title bar", async () => {
    useLayoutStore.setState({ assistantOpen: false });
    const user = userEvent.setup();
    render(<App />);

    expect(screen.queryByLabelText("Personal assistant")).toBeNull();
    const toggle = screen.getByLabelText("Assistant");
    await user.click(toggle);
    expect(screen.getByLabelText("Personal assistant")).toBeInTheDocument();
    expect(toggle).toHaveAttribute("aria-pressed", "true");

    await user.click(screen.getByLabelText("Hide assistant"));
    expect(screen.queryByLabelText("Personal assistant")).toBeNull();
  });

  it("turns the assistant off and on from the title bar", async () => {
    useLayoutStore.setState({ assistantOpen: true });
    const user = userEvent.setup();
    render(<App />);

    assistant.setResult = false;
    await user.click(screen.getByLabelText("Turn assistant off"));
    expect(invoke).toHaveBeenCalledWith("assistant_enabled_set", {
      enabled: false,
    });
    expect(await screen.findByText("The assistant is off")).toBeInTheDocument();
    expect(screen.queryByPlaceholderText("Message the assistant…")).toBeNull();

    assistant.setResult = true;
    await user.click(screen.getByLabelText("Turn assistant on"));
    expect(invoke).toHaveBeenCalledWith("assistant_enabled_set", {
      enabled: true,
    });
    expect(
      await screen.findByPlaceholderText("Message the assistant…"),
    ).toBeInTheDocument();
  });

  it("closes the open Git board when its project closes", async () => {
    useLayoutStore.setState({ openBoard: { projectId: "proj-1" } });
    render(<App />);
    const handler = () =>
      vi
        .mocked(listen)
        .mock.calls.find(([name]) => name === "project:closed")?.[1];
    await waitFor(() => expect(handler()).toBeDefined());
    const fire = (projectId: string) =>
      handler()?.({ event: "project:closed", id: 0, payload: { projectId } });

    fire("proj-2");
    expect(useLayoutStore.getState().openBoard).toEqual({
      projectId: "proj-1",
    });
    fire("proj-1");
    expect(useLayoutStore.getState().openBoard).toBeNull();
  });

  it("draws its own window controls on Windows", () => {
    platform.windows = true;
    render(<App />);

    expect(screen.getByLabelText("Minimize")).toBeInTheDocument();
    expect(screen.getByLabelText("Close")).toBeInTheDocument();
    expect(screen.getByLabelText("Settings")).toBeInTheDocument();
  });

  it("leaves the window controls to the native traffic lights on macOS", () => {
    platform.mac = true;
    render(<App />);

    expect(screen.queryByLabelText("Minimize")).toBeNull();
    expect(screen.queryByLabelText("Close")).toBeNull();
    // Settings lives in the native menu bar (⌘,) there.
    expect(screen.queryByLabelText("Settings")).toBeNull();
    expect(screen.getByText("Podium").closest("header")).toHaveAttribute(
      "data-tauri-drag-region",
      "true",
    );
  });
});

describe("App — assistant sub-agents", () => {
  const subagent = {
    id: "sub-1",
    name: "Repo sweep",
    kind: "full",
    task: "Repo sweep",
    phase: "running",
    status: { state: "running", pid: 7, since: "2026-01-01T10:00:00Z" },
    createdAt: "2026-01-01T10:00:00Z",
    startedAt: "2026-01-01T10:00:00Z",
    endedAt: null,
    result: null,
    error: null,
  };

  it("loads them on start, panel closed or not", async () => {
    useLayoutStore.setState({ assistantOpen: false });
    assistant.subagents = [subagent];
    render(<App />);

    await waitFor(() =>
      expect(useAssistantStore.getState().subagents).toHaveLength(1),
    );
    expect(invoke).toHaveBeenCalledWith("assistant_subagents_list");
  });

  it("shows an opened sub-agent's terminal in the work area", async () => {
    assistant.subagents = [subagent];
    useLayoutStore.setState({ openSubagent: "sub-1" });
    render(<App />);

    expect(
      await screen.findByText("Assistant sub-agent · full"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Welcome to Podium")).toBeNull();
  });

  it("applies process status events to a sub-agent", async () => {
    assistant.subagents = [subagent];
    render(<App />);
    await waitFor(() =>
      expect(useAssistantStore.getState().subagents).toHaveLength(1),
    );
    const handler = () =>
      vi
        .mocked(listen)
        .mock.calls.find(([name]) => name === "process:status")?.[1];
    await waitFor(() => expect(handler()).toBeDefined());

    const status = { state: "stopping" };
    handler()?.({
      event: "process:status",
      id: 0,
      payload: { projectId: "internal", processId: "sub-1", status },
    });
    expect(useAssistantStore.getState().subagents[0].status).toEqual(status);
  });
});
