import { beforeEach, describe, expect, it } from "vitest";

import { reparentTerminalElement } from "./terminalRegistry";

// `attachToElement` itself needs a real browser (xterm's `terminal.open`
// touches `matchMedia`/canvas, which jsdom lacks), so we test the pure DOM
// reparenting it delegates to — this is the exact path that regressed when
// switching between terminals/agents left the previous one visible.
describe("reparentTerminalElement", () => {
  let host: HTMLDivElement;

  beforeEach(() => {
    document.body.replaceChildren();
    host = document.createElement("div");
    document.body.appendChild(host);
  });

  it("moves the terminal element into the host", () => {
    const el = document.createElement("div");
    reparentTerminalElement(el, host);
    expect(el.parentNode).toBe(host);
    expect(host.childElementCount).toBe(1);
  });

  it("evicts a previous terminal when a reused host already holds one", () => {
    const first = document.createElement("div");
    const second = document.createElement("div");
    reparentTerminalElement(first, host);
    reparentTerminalElement(second, host);

    // The host must hold *only* the current terminal — the stacking bug was
    // the old element lingering alongside the new one.
    expect(host.childElementCount).toBe(1);
    expect(host.firstElementChild).toBe(second);
    expect(first.parentNode).toBeNull();
  });

  it("is a no-op when the element is already the sole child", () => {
    const el = document.createElement("div");
    host.appendChild(el);
    reparentTerminalElement(el, host);
    expect(host.firstElementChild).toBe(el);
    expect(host.childElementCount).toBe(1);
  });
});
