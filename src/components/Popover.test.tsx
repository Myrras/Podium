import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { Popover, placePopover } from "./Popover";

const vp = { w: 1000, h: 800 };
const size = { w: 200, h: 100 };

describe("placePopover", () => {
  it("centres 8px under the anchor", () => {
    expect(placePopover({ x: 400, y: 100, w: 100, h: 20 }, size, vp)).toEqual({
      left: 350,
      top: 128,
      above: false,
    });
  });
  it("goes above when below does not fit", () => {
    expect(placePopover({ x: 400, y: 700, w: 100, h: 20 }, size, vp)).toEqual({
      left: 350,
      top: 592,
      above: true,
    });
  });
  it("stays 6px inside the window", () => {
    expect(placePopover({ x: 0, y: 100, w: 10, h: 20 }, size, vp).left).toBe(6);
    expect(placePopover({ x: 990, y: 100, w: 10, h: 20 }, size, vp).left).toBe(
      794,
    );
  });
});

function setup() {
  const anchor = document.createElement("button");
  document.body.append(anchor);
  const onClose = vi.fn();
  render(
    <Popover anchor={anchor} title="Items" onClose={onClose}>
      <button>inside</button>
    </Popover>,
  );
  return { anchor, onClose };
}

describe("Popover", () => {
  it("renders nothing without an anchor", () => {
    render(
      <Popover anchor={null} title="x" onClose={() => {}}>
        body
      </Popover>,
    );
    expect(screen.queryByText("body")).toBeNull();
  });

  it("closes on ×", () => {
    const { onClose } = setup();
    fireEvent.click(screen.getByLabelText("Close"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closes on Esc inside, without bubbling", () => {
    const { onClose } = setup();
    const outer = vi.fn();
    document.addEventListener("keydown", outer);
    fireEvent.keyDown(screen.getByText("inside"), { key: "Escape" });
    document.removeEventListener("keydown", outer);
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(outer).not.toHaveBeenCalled();
  });

  it("closes on outside mousedown only", () => {
    const { anchor, onClose } = setup();
    const dialog = document.createElement("div");
    dialog.setAttribute("role", "dialog");
    document.body.append(dialog);
    fireEvent.mouseDown(screen.getByText("inside"));
    fireEvent.mouseDown(anchor);
    fireEvent.mouseDown(dialog);
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.mouseDown(document.body);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("shows a back button when onBack is given", () => {
    const anchor = document.createElement("button");
    document.body.append(anchor);
    const onBack = vi.fn();
    render(
      <Popover
        anchor={anchor}
        title="t"
        onClose={() => {}}
        onBack={onBack}
        backLabel="Back to To-dos"
      >
        x
      </Popover>,
    );
    fireEvent.click(screen.getByText("Back to To-dos"));
    expect(onBack).toHaveBeenCalled();
  });
});
