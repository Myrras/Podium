import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// App wires up IPC on mount; jsdom has no Tauri bridge, so stub it out.
// `invoke` resolves per-command fixtures (all list commands — including
// `workspace_list` — return arrays) and `listen` resolves a no-op unlisten.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) =>
    Promise.resolve(
      cmd === "adapters_list"
        ? [{ id: "claude-code", displayName: "Claude Code", available: true }]
        : [],
    ),
  ),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve(null)),
}));
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

import { useLayoutStore } from "./state/layoutStore";
import App from "./App";

describe("App", () => {
  beforeEach(() => {
    platform.mac = false;
    platform.windows = false;
  });

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
