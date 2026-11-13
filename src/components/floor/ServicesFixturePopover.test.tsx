import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));

import type { ProcessInfo, ProjectInfo } from "../../ipc/types";
import { useFloorStore } from "../../state/floorStore";
import { useProcessStore } from "../../state/processStore";
import { useProjectStore } from "../../state/projectStore";
import { ServicesFixturePopover } from "./ServicesFixturePopover";

const svc = (id: string, name: string, state: string, projectId = "p1") =>
  ({
    id,
    projectId,
    name,
    kind: { kind: "service" },
    status:
      state === "running"
        ? { state: "running", pid: 1 }
        : { state: "notStarted" },
  }) as unknown as ProcessInfo;

const reload = vi.fn(() => Promise.resolve());
const project = (configError: string | null) =>
  ({ id: "p1", name: "Demo", configError }) as unknown as ProjectInfo;

function setup(configError: string | null = null) {
  useProjectStore.setState(
    { projects: [project(configError)], reloadProjectConfig: reload },
    false,
  );
  useProcessStore.setState(
    {
      processes: [
        svc("s1", "web", "running"),
        svc("s2", "db", "off"),
        svc("s3", "other", "running", "p2"),
      ],
    },
    false,
  );
  render(
    <ServicesFixturePopover
      projectId="p1"
      itemId={null}
      anchor={document.body}
    />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  useFloorStore.getState().reset();
});

describe("ServicesFixturePopover", () => {
  it("lists this project's services with a status word", () => {
    setup();
    expect(screen.getByText("Services")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /web/ })).toHaveTextContent("Up");
    expect(screen.getByRole("button", { name: /db/ })).toHaveTextContent(
      "Not started",
    );
    expect(screen.queryByText("other")).toBeNull();
  });

  it("opens a service in the modal", () => {
    setup();
    fireEvent.click(screen.getByRole("button", { name: /web/ }));
    expect(useFloorStore.getState().terminalId).toBe("s1");
  });

  it("reloads podium.yml and shows a config error", () => {
    setup("bad yaml");
    expect(screen.getByRole("alert")).toHaveTextContent("bad yaml");
    fireEvent.click(screen.getByRole("button", { name: "Reload podium.yml" }));
    expect(reload).toHaveBeenCalledWith("p1");
  });
});
