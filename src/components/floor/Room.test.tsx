import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ProcessInfo, ProjectInfo } from "../../ipc/types";
import type { RoomModel } from "../../lib/floor/types";
import { Room } from "./Room";

vi.mock("./RoomNameplate", () => ({
  RoomNameplate: () => <i data-testid="plate" />,
}));
vi.mock("./RoomWall", () => ({ RoomWall: () => <i data-testid="wall" /> }));
vi.mock("./Desk", () => ({
  Desk: ({ process }: { process: ProcessInfo }) => (
    <i data-testid="desk" data-id={process.id} />
  ),
}));

const proc = (id: string) => ({ id }) as unknown as ProcessInfo;
const model = (
  over: Partial<RoomModel> = {},
  color: string | null = "#58a6ff",
) =>
  ({
    project: { id: "p1", color } as unknown as ProjectInfo,
    rect: { x: 40, y: 80, w: 400, h: 300 },
    agents: [proc("a1"), proc("a2")],
    terminals: [proc("t1")],
    services: [],
    needs: 0,
    crashed: 0,
    lit: true,
    ...over,
  }) as RoomModel;

describe("Room", () => {
  it("positions at the rect and renders plate, wall, agents then terminals", () => {
    const { container, getAllByTestId, getByTestId } = render(
      <Room room={model()} />,
    );
    const root = container.firstElementChild as HTMLElement;
    expect(root.style.left).toBe("40px");
    expect(root.style.top).toBe("80px");
    expect(root.style.width).toBe("400px");
    expect(root.style.getPropertyValue("--pc")).toBe("#58a6ff");
    expect(root.dataset.room).toBe("p1");
    expect(root.dataset.lit).toBe("true");
    expect(getByTestId("plate")).toBeTruthy();
    expect(getByTestId("wall")).toBeTruthy();
    expect(getAllByTestId("desk").map((d) => d.dataset.id)).toEqual([
      "a1",
      "a2",
      "t1",
    ]);
  });

  it("marks unlit and uncoloured rooms", () => {
    const { container } = render(<Room room={model({ lit: false }, null)} />);
    const root = container.firstElementChild as HTMLElement;
    expect(root.dataset.lit).toBe("false");
    expect(root.dataset.tinted).toBe("false");
  });
});
