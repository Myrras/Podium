import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { Popover } from "../components/Popover";
import { useFloorStore } from "../state/floorStore";
import { followCamera } from "./floor/follow";
import { useContextMenu } from "./useContextMenu";

function Row({ onOuter }: { onOuter: () => void }) {
  const menu = useContextMenu();
  return (
    <div onContextMenu={onOuter}>
      <span
        onContextMenu={(e) =>
          menu.open(e, [{ label: "Archive", onSelect: vi.fn() }])
        }
      >
        Row
      </span>
      {menu.element}
    </div>
  );
}

describe("useContextMenu", () => {
  it("opens one menu at the pointer, stops the event and closes on Esc", () => {
    const outer = vi.fn();
    render(<Row onOuter={outer} />);
    expect(screen.queryByRole("menu")).toBeNull();
    const ev = fireEvent.contextMenu(screen.getByText("Row"), {
      clientX: 10,
      clientY: 20,
    });
    expect(ev).toBe(false);
    expect(outer).not.toHaveBeenCalled();
    expect(screen.getByRole("menuitem", { name: "Archive" })).toBeVisible();
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("inside a Popover, closes when the popover's follow moves", () => {
    const anchor = document.createElement("button");
    document.body.append(anchor);
    render(
      <Popover
        anchor={anchor}
        title="t"
        onClose={vi.fn()}
        follow={followCamera}
      >
        <Row onOuter={vi.fn()} />
      </Popover>,
    );
    fireEvent.contextMenu(screen.getByText("Row"));
    expect(screen.getByRole("menu")).toBeInTheDocument();
    act(() => useFloorStore.getState().setCamera({ x: 5, y: 0, z: 1 }));
    expect(screen.queryByRole("menu")).toBeNull();
    useFloorStore.getState().reset();
    anchor.remove();
  });
});
