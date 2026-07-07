import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The project store pulls in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve(null)),
}));

import { useProjectStore } from "../state/projectStore";
import { WelcomeScreen } from "./WelcomeScreen";

const initialState = useProjectStore.getState();

function recent(name: string, path: string) {
  return { name, path, lastOpenedAt: 1 };
}

describe("WelcomeScreen", () => {
  beforeEach(() => {
    // Full replace restores the original actions after tests stub them.
    useProjectStore.setState(initialState, true);
  });

  it("lists recent projects while no project is added", () => {
    useProjectStore.setState({
      projects: [],
      recents: [recent("Webshop", "/tmp/webshop"), recent("Blog", "/tmp/blog")],
    });
    render(<WelcomeScreen />);

    expect(screen.getByText("Recent projects")).toBeInTheDocument();
    expect(screen.getByText("Webshop")).toBeInTheDocument();
    expect(screen.getByText("/tmp/webshop")).toBeInTheDocument();
    expect(screen.getByText("Blog")).toBeInTheDocument();
  });

  it("hides the recents list when there are none", () => {
    useProjectStore.setState({ projects: [], recents: [] });
    render(<WelcomeScreen />);

    expect(screen.queryByText("Recent projects")).not.toBeInTheDocument();
  });

  it("opens a recent project on click", () => {
    const openProject = vi.fn(() => Promise.resolve());
    useProjectStore.setState({
      projects: [],
      recents: [recent("Webshop", "/tmp/webshop")],
      openProject,
    });
    render(<WelcomeScreen />);

    fireEvent.click(screen.getByTitle("/tmp/webshop"));
    expect(openProject).toHaveBeenCalledWith("/tmp/webshop");
  });

  it("removes a recent via its remove button", () => {
    const removeRecent = vi.fn(() => Promise.resolve());
    useProjectStore.setState({
      projects: [],
      recents: [recent("Webshop", "/tmp/webshop")],
      removeRecent,
    });
    render(<WelcomeScreen />);

    fireEvent.click(
      screen.getByLabelText("Remove Webshop from recent projects"),
    );
    expect(removeRecent).toHaveBeenCalledWith("/tmp/webshop");
  });
});
