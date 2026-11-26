import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { Modal } from "./Modal";

describe("Modal", () => {
  it("renders outside its parent so clipped ancestors can't cut it off", () => {
    const { container } = render(
      <div style={{ overflow: "hidden" }}>
        <Modal open title="Hello" onClose={() => {}}>
          body
        </Modal>
      </div>,
    );
    const dialog = screen.getByRole("dialog", { name: "Hello" });
    expect(container.contains(dialog)).toBe(false);
    expect(document.body.contains(dialog)).toBe(true);
  });
});

describe("Modal sizeKey", () => {
  afterEach(() => {
    localStorage.removeItem("podium.size.sample");
    vi.unstubAllGlobals();
  });

  function sized(sizeKey?: string) {
    render(
      <Modal open title="Sized" onClose={() => {}} sizeKey={sizeKey}>
        body
      </Modal>,
    );
    return screen.getByRole("dialog");
  }

  it("without it: not resizable, the width via the variable only", () => {
    const el = sized();
    expect(el).not.toHaveAttribute("data-resizable");
    expect(el.style.getPropertyValue("--modal-w")).toBe("460px");
    expect(el.style.width).toBe("");
  });

  it("restores the saved size", () => {
    localStorage.setItem("podium.size.sample", '{"w":500,"h":400}');
    const el = sized("sample");
    expect(el).toHaveAttribute("data-resizable");
    expect(el.style.width).toBe("500px");
    expect(el.style.height).toBe("400px");
  });

  it("a saved size bigger than the window falls back to the default", () => {
    localStorage.setItem(
      "podium.size.sample",
      JSON.stringify({ w: window.innerWidth, h: 400 }),
    );
    const el = sized("sample");
    expect(el.style.width).toBe("");
    expect(el.style.height).toBe("");
    expect(el).not.toHaveAttribute("data-sized");
  });

  it("saves a dragged size", () => {
    const callbacks: (() => void)[] = [];
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(cb: () => void) {
          callbacks.push(cb);
        }
        observe() {}
        disconnect() {}
      },
    );
    const el = sized("sample");
    const fire = () => act(() => callbacks.forEach((cb) => cb()));
    fire();
    expect(localStorage.getItem("podium.size.sample")).toBeNull();

    el.style.width = "450px";
    el.style.height = "300px";
    Object.defineProperty(el, "offsetWidth", { value: 450 });
    Object.defineProperty(el, "offsetHeight", { value: 300 });
    fire();
    expect(JSON.parse(localStorage.getItem("podium.size.sample")!)).toEqual({
      w: 450,
      h: 300,
    });
    expect(el).toHaveAttribute("data-sized");
  });
});
