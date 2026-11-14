import { fireEvent, render, screen } from "@testing-library/react";
import { createPortal } from "react-dom";
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { followCamera } from "../lib/floor/follow";
import { useFloorStore } from "../state/floorStore";
import { Popover, PopoverSlot, placePopover } from "./Popover";

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

  it("ignores Esc an inline editor already handled", () => {
    const anchor = document.createElement("button");
    document.body.append(anchor);
    const onClose = vi.fn();
    render(
      <Popover anchor={anchor} title="t" onClose={onClose}>
        <input
          aria-label="edit"
          onKeyDown={(e) => {
            if (e.key === "Escape") e.preventDefault();
          }}
        />
      </Popover>,
    );
    fireEvent.keyDown(screen.getByLabelText("edit"), { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("ignores Esc from a portalled child and lets it reach document", () => {
    const anchor = document.createElement("button");
    document.body.append(anchor);
    const onClose = vi.fn();
    render(
      <Popover anchor={anchor} title="t" onClose={onClose}>
        {createPortal(<button>in modal</button>, document.body)}
      </Popover>,
    );
    const outer = vi.fn();
    document.addEventListener("keydown", outer);
    fireEvent.keyDown(screen.getByText("in modal"), { key: "Escape" });
    document.removeEventListener("keydown", outer);
    expect(onClose).not.toHaveBeenCalled();
    expect(outer).toHaveBeenCalledTimes(1);
  });

  it("closes when the anchor is no longer in the document", () => {
    const anchor = document.createElement("button");
    const onClose = vi.fn();
    render(
      <Popover anchor={anchor} title="t" onClose={onClose}>
        x
      </Popover>,
    );
    expect(onClose).toHaveBeenCalled();
  });

  it("keeps open on mousedown inside [data-popover-keep]", () => {
    const { onClose } = setup();
    const map = document.createElement("div");
    map.setAttribute("data-popover-keep", "");
    const inner = document.createElement("span");
    map.append(inner);
    document.body.append(map);
    fireEvent.mouseDown(inner);
    expect(onClose).not.toHaveBeenCalled();
    map.remove();
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

  it("focuses itself on open, so Esc works at once", () => {
    const { onClose } = setup();
    const root = screen.getByRole("group");
    expect(document.activeElement).toBe(root);
    fireEvent.keyDown(document.activeElement as Element, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("returns focus to the anchor on close", () => {
    const anchor = document.createElement("button");
    document.body.append(anchor);
    const { unmount } = render(
      <Popover anchor={anchor} title="t" onClose={() => {}}>
        x
      </Popover>,
    );
    expect(document.activeElement).not.toBe(anchor);
    unmount();
    expect(document.activeElement).toBe(anchor);
  });

  it("shows the count after the title", () => {
    const anchor = document.createElement("button");
    document.body.append(anchor);
    render(
      <Popover anchor={anchor} title="To-dos" count={5} onClose={() => {}}>
        x
      </Popover>,
    );
    expect(screen.getByRole("heading")).toHaveTextContent("To-dos5");
  });

  it("PopoverSlot renders into the header and footer", () => {
    const anchor = document.createElement("button");
    document.body.append(anchor);
    render(
      <Popover anchor={anchor} title="t" onClose={() => {}}>
        <PopoverSlot name="header">
          <button>New</button>
        </PopoverSlot>
        <PopoverSlot name="footer">
          <button>Archive</button>
        </PopoverSlot>
      </Popover>,
    );
    expect(document.querySelector("header")).toContainElement(
      screen.getByText("New"),
    );
    expect(document.querySelector("footer")).toContainElement(
      screen.getByText("Archive"),
    );
  });

  it("PopoverSlot renders in place outside a Popover", () => {
    render(
      <div data-testid="host">
        <PopoverSlot name="header">
          <button>New</button>
        </PopoverSlot>
      </div>,
    );
    expect(screen.getByTestId("host")).toContainElement(
      screen.getByText("New"),
    );
  });
});

describe("Popover placement", () => {
  afterEach(() => {
    vi.useRealTimers();
    useFloorStore.getState().reset();
  });

  /** An anchor whose box the test moves; jsdom has no layout. */
  function anchored(follow: typeof followCamera | null = followCamera) {
    vi.useFakeTimers({ toFake: ["requestAnimationFrame", "performance"] });
    const raf = vi.spyOn(window, "requestAnimationFrame");
    const anchor = document.createElement("button");
    document.body.append(anchor);
    let x = 100;
    anchor.getBoundingClientRect = () =>
      ({ x, y: 50, width: 40, height: 20 }) as DOMRect;
    render(
      <Popover
        anchor={anchor}
        title="t"
        onClose={() => {}}
        follow={follow ?? undefined}
      >
        x
      </Popover>,
    );
    const left = () => screen.getByRole("group").style.left;
    return { raf, left, moveTo: (to: number) => (x = to) };
  }

  it("places once and runs no frame loop while idle", () => {
    const { raf, left } = anchored();
    expect(left()).toBe("120px");
    vi.advanceTimersByTime(1000);
    expect(raf).not.toHaveBeenCalled();
  });

  it("follows an animated camera jump for its transition, then stops", () => {
    const { raf, left, moveTo } = anchored();
    moveTo(300);
    act(() =>
      useFloorStore
        .getState()
        .setCamera({ x: 1, y: 0, z: 1 }, { animate: true }),
    );
    vi.advanceTimersByTime(20);
    expect(left()).toBe("320px");
    moveTo(400);
    vi.advanceTimersByTime(100);
    expect(left()).toBe("420px");
    vi.advanceTimersByTime(400);
    const calls = raf.mock.calls.length;
    moveTo(500);
    vi.advanceTimersByTime(500);
    expect(raf.mock.calls.length).toBe(calls);
    expect(left()).toBe("420px");
  });

  it("re-places one frame after a pan, and on window resize", () => {
    const { raf, left, moveTo } = anchored();
    moveTo(200);
    act(() => useFloorStore.getState().setCamera({ x: 5, y: 0, z: 1 }));
    act(() => useFloorStore.getState().setCamera({ x: 6, y: 0, z: 1 }));
    // Several pan moves in one frame share one frame.
    expect(raf).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(20);
    expect(left()).toBe("220px");
    moveTo(250);
    fireEvent(window, new Event("resize"));
    expect(left()).toBe("270px");
    expect(raf).toHaveBeenCalledTimes(1);
  });

  it("ignores the camera without a follow", () => {
    const { raf, left, moveTo } = anchored(null);
    moveTo(300);
    act(() => useFloorStore.getState().setCamera({ x: 5, y: 0, z: 1 }));
    vi.advanceTimersByTime(20);
    expect(raf).not.toHaveBeenCalled();
    expect(left()).toBe("120px");
  });
});
