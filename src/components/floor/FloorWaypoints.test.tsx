import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProjectId } from "../../ipc/types";
import type { RoomModel } from "../../lib/floor/types";
import { useFloorStore } from "../../state/floorStore";
import { FloorWaypoints } from "./FloorWaypoints";

const jumpTo = vi.hoisted(() => vi.fn());
vi.mock("../../lib/floor/camera", () => ({ jumpTo }));
vi.mock("../../lib/floor/rooms", () => ({
  useRooms: () => [
    {
      project: {
        id: "p1" as ProjectId,
        name: "Alpha",
        iconInitials: "AL",
        color: null,
      },
      rect: { x: 3000, y: 0, w: 400, h: 400 },
      needs: 2,
      crashed: 1,
    } as unknown as RoomModel,
  ],
}));

describe("FloorWaypoints", () => {
  beforeEach(() => {
    jumpTo.mockClear();
    useFloorStore.setState(
      { ...useFloorStore.getState(), viewport: { w: 1000, h: 600 } },
      true,
    );
  });

  it("renders a marker that jumps to the room", () => {
    render(<FloorWaypoints />);
    const btn = screen.getByRole("button", {
      name: "Go to Alpha: 2 agents waiting on you, 1 crashed",
    });
    fireEvent.click(btn);
    expect(jumpTo).toHaveBeenCalledWith("p1");
  });
});
