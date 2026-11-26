import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
vi.mock("../GitBoard", () => ({
  GitBoard: ({ projectId }: { projectId: string }) => (
    <div>
      Board of {projectId}
      <input
        type="search"
        aria-label="Search board"
        onKeyDown={(e) => {
          if (e.key === "Escape") e.preventDefault();
        }}
      />
    </div>
  ),
}));

import { useIssueStore } from "../../state/issueStore";
import { useLayoutStore } from "../../state/layoutStore";
import { FloorBoardModal } from "./FloorBoardModal";

const open = () =>
  act(() => useLayoutStore.setState({ openBoard: { projectId: "proj-1" } }));

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.removeItem("podium.size.board");
  useIssueStore.setState({
    startAutoRefresh: vi.fn(() => () => undefined),
    loadOverview: vi.fn(() => Promise.resolve()),
  });
  useLayoutStore.setState({ openBoard: null });
});

describe("FloorBoardModal", () => {
  it("renders nothing while no board is open", () => {
    render(<FloorBoardModal />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows the board and closes on a click beside the card, not on it", () => {
    render(<FloorBoardModal />);
    open();
    expect(screen.getByRole("dialog", { name: "Git board" })).toHaveTextContent(
      "Board of proj-1",
    );
    fireEvent.mouseDown(screen.getByRole("dialog"));
    expect(useLayoutStore.getState().openBoard).not.toBeNull();
    fireEvent.mouseDown(screen.getByRole("presentation"));
    expect(useLayoutStore.getState().openBoard).toBeNull();
  });

  it("closes on Esc, but not on Esc in the search field", () => {
    render(<FloorBoardModal />);
    open();
    fireEvent.keyDown(screen.getByLabelText("Search board"), { key: "Escape" });
    expect(useLayoutStore.getState().openBoard).not.toBeNull();
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(useLayoutStore.getState().openBoard).toBeNull();
  });

  it("opens at its saved size, or the default when that no longer fits", () => {
    localStorage.setItem("podium.size.board", '{"w":700,"h":500}');
    const { unmount } = render(<FloorBoardModal />);
    open();
    expect(screen.getByRole("dialog").style.width).toBe("700px");
    expect(screen.getByRole("dialog").style.height).toBe("500px");
    unmount();

    localStorage.setItem(
      "podium.size.board",
      JSON.stringify({ w: 700, h: window.innerHeight }),
    );
    render(<FloorBoardModal />);
    expect(screen.getByRole("dialog").style.width).toBe("");
  });
});
