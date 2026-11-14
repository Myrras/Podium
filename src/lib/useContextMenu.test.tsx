import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

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
});
