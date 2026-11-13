import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
vi.mock("../GitBoard", () => ({
  GitBoard: ({ projectId }: { projectId: string }) => (
    <div>Board of {projectId}</div>
  ),
}));
vi.mock("./FloorMap", () => ({ FloorMap: () => <div>Map</div> }));
vi.mock("./FloorWaypoints", () => ({
  FloorWaypoints: () => <div>Waypoints</div>,
}));

import { useFloorStore } from "../../state/floorStore";
import { useIssueStore } from "../../state/issueStore";
import { useLayoutStore } from "../../state/layoutStore";
import { Floor } from "./Floor";

const stopRefresh = vi.fn();
const startAutoRefresh = vi.fn((_id: string) => stopRefresh);
const loadOverview = vi.fn(() => Promise.resolve());

beforeEach(() => {
  vi.clearAllMocks();
  useIssueStore.setState({ startAutoRefresh, loadOverview });
  useLayoutStore.setState({ openBoard: null });
  useFloorStore.getState().reset();
});

describe("Floor", () => {
  it("shows map and waypoints in one box", () => {
    render(<Floor />);
    expect(screen.getByText("Map").parentElement).toBe(
      screen.getByText("Waypoints").parentElement,
    );
    expect(screen.queryByText(/Board of/)).toBeNull();
  });

  it("shows the Git board in place of the map when one is open", () => {
    useLayoutStore.setState({ openBoard: { projectId: "proj-1" } });
    render(<Floor />);
    expect(screen.getByText("Board of proj-1")).toBeInTheDocument();
    expect(screen.queryByText("Map")).toBeNull();
  });

  it("keeps the open board refreshing and stops when it closes", () => {
    render(<Floor />);
    expect(startAutoRefresh).not.toHaveBeenCalled();

    act(() => useLayoutStore.setState({ openBoard: { projectId: "proj-1" } }));
    expect(startAutoRefresh).toHaveBeenCalledWith("proj-1");
    expect(loadOverview).toHaveBeenCalledTimes(1);

    act(() => useLayoutStore.setState({ openBoard: null }));
    expect(stopRefresh).toHaveBeenCalledTimes(1);
  });

  it("resets the floor store on unmount", () => {
    const { unmount } = render(<Floor />);
    useFloorStore.getState().openTerminal("proc-1");
    unmount();
    expect(useFloorStore.getState().terminalId).toBeNull();
  });
});
