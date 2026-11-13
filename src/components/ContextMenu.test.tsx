import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ContextMenu, clampMenu } from "./ContextMenu";
import type { MenuItem } from "./ContextMenu";

const vp = { w: 1000, h: 800 };
const size = { w: 200, h: 100 };

describe("clampMenu", () => {
  it("keeps the point when it fits", () => {
    expect(clampMenu(100, 100, size, vp)).toEqual({ left: 100, top: 100 });
  });
  it("stays 6px inside every edge", () => {
    expect(clampMenu(-5, -5, size, vp)).toEqual({ left: 6, top: 6 });
    expect(clampMenu(990, 790, size, vp)).toEqual({ left: 794, top: 694 });
  });
});

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

  it("does not run a disabled item", () => {
    const { onClose } = setup({ disabled: true });
    fireEvent.click(screen.getByText("Off"));
    expect(onClose).not.toHaveBeenCalled();
  });
});
