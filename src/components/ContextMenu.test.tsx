import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { act } from "react";

import { followCamera } from "../lib/floor/follow";
import { useFloorStore } from "../state/floorStore";
import { ContextMenu } from "./ContextMenu";
import type { MenuItem } from "./ContextMenu";

function setup(
  extra: Partial<{ disabled: boolean }> = {},
  onParentKey?: () => void,
) {
  const a = vi.fn();
  const b = vi.fn();
  const onClose = vi.fn();
  const items: MenuItem[] = [
    { label: "Off", onSelect: vi.fn(), disabled: extra.disabled },
    { label: "A", onSelect: a },
    "separator",
    { label: "B", onSelect: b, danger: true },
  ];
  render(
    <div onKeyDown={onParentKey}>
      <ContextMenu x={10} y={10} items={items} onClose={onClose} />
    </div>,
  );
  return { a, b, onClose };
}

describe("ContextMenu", () => {
  it("renders roles and focuses the first enabled item", () => {
    setup({ disabled: true });
    expect(screen.getByRole("menu")).toHaveAttribute("data-popover-keep");
    expect(screen.getAllByRole("menuitem")).toHaveLength(3);
    expect(screen.getByRole("separator")).toBeInTheDocument();
    expect(screen.getByText("A")).toHaveFocus();
  });

  it("moves focus with the arrows, wrapping", () => {
    setup({ disabled: true });
    const menu = screen.getByRole("menu");
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(screen.getByText("B")).toHaveFocus();
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(screen.getByText("A")).toHaveFocus();
    fireEvent.keyDown(menu, { key: "ArrowUp" });
    expect(screen.getByText("B")).toHaveFocus();
  });

  it("activates with Enter and closes first", async () => {
    const { a, onClose } = setup();
    const order: string[] = [];
    onClose.mockImplementation(() => order.push("close"));
    a.mockImplementation(() => order.push("select"));
    screen.getByText("A").focus();
    await userEvent.keyboard("{Enter}");
    expect(order).toEqual(["close", "select"]);
  });

  it("closes on Esc without the event bubbling", () => {
    const onWin = vi.fn();
    const { onClose } = setup({}, onWin);
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onWin).not.toHaveBeenCalled();
  });

  it("closes on an outside mousedown, any button, not an inside one", () => {
    const { onClose } = setup();
    fireEvent.mouseDown(screen.getByText("A"));
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.mouseDown(document.body, { button: 2 });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closes on a wheel outside it and on a window resize, not on a wheel inside", () => {
    const { onClose } = setup();
    fireEvent.wheel(screen.getByText("A"));
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.wheel(document.body);
    expect(onClose).toHaveBeenCalledTimes(1);
    fireEvent(window, new Event("resize"));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("closes on each move of what it follows (the floor camera)", () => {
    const onClose = vi.fn();
    const { unmount } = render(
      <ContextMenu
        x={10}
        y={10}
        items={[{ label: "A", onSelect: vi.fn() }]}
        onClose={onClose}
        follow={followCamera}
      />,
    );
    act(() => useFloorStore.getState().setCamera({ x: 5, y: 0, z: 1 }));
    expect(onClose).toHaveBeenCalledTimes(1);
    unmount();
    act(() => useFloorStore.getState().setCamera({ x: 9, y: 0, z: 1 }));
    expect(onClose).toHaveBeenCalledTimes(1);
    useFloorStore.getState().reset();
  });

  it("does not run a disabled item", () => {
    const { onClose } = setup({ disabled: true });
    fireEvent.click(screen.getByText("Off"));
    expect(onClose).not.toHaveBeenCalled();
  });
});

describe("ContextMenu confirm", () => {
  function setupConfirm() {
    const onSelect = vi.fn();
    const onClose = vi.fn();
    const items: MenuItem[] = [
      {
        label: "Stop",
        onSelect,
        confirm: { title: "Stop x?", body: "It ends.", action: "Stop now" },
      },
    ];
    render(<ContextMenu x={10} y={10} items={items} onClose={onClose} />);
    fireEvent.click(screen.getByText("Stop"));
    return { onSelect, onClose };
  }

  it("shows the confirm view without running the item", () => {
    const { onSelect, onClose } = setupConfirm();
    expect(screen.getByText("Stop x?")).toBeInTheDocument();
    expect(screen.getByText("It ends.")).toBeInTheDocument();
    expect(onSelect).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("focuses Cancel", () => {
    setupConfirm();
    expect(screen.getByText("Cancel")).toHaveFocus();
  });

  it("Cancel returns to the list", () => {
    setupConfirm();
    fireEvent.click(screen.getByText("Cancel"));
    expect(screen.getByRole("menuitem", { name: "Stop" })).toBeInTheDocument();
  });

  it("first Esc returns to the list, second closes", () => {
    const { onClose } = setupConfirm();
    fireEvent.keyDown(screen.getByRole("alertdialog"), { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("the action selects and closes", () => {
    const { onSelect, onClose } = setupConfirm();
    fireEvent.click(screen.getByText("Stop now"));
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
