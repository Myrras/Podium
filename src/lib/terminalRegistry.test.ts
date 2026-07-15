import { beforeEach, describe, expect, it } from "vitest";

import { proposeGrid, reparentTerminalElement } from "./terminalRegistry";

// `fitTerminal` needs a real renderer (xterm cell metrics), so we test the
// pure grid math it delegates to. The regression this guards: the proposal
// must depend only on host size and device cell size — never on the current
// grid — so refitting an unchanged host never churns the terminal.
describe("proposeGrid", () => {
  // 33 device px per row at dpr 2 = 16.5 CSS px — the fractional cell height
  // that made FitAddon's css-based math flip-flop.
  const cell = { width: 17, height: 33 };

  it("floors the grid to whole cells that fit the content box", () => {
    // 330 CSS px * dpr 2 = 660 device px = exactly 20 rows of 33.
    expect(proposeGrid(340, 330, cell, 2)).toEqual({ cols: 40, rows: 20 });
    // One CSS px less: 658 device px no longer fits 20 rows.
    expect(proposeGrid(340, 329, cell, 2)).toEqual({ cols: 40, rows: 19 });
  });

  it("is stable: same inputs always yield the same grid", () => {
    const first = proposeGrid(512.4, 387.6, cell, 2);
    expect(proposeGrid(512.4, 387.6, cell, 2)).toEqual(first);
  });

  it("never exceeds the available height", () => {
    for (let avail = 50; avail < 400; avail += 7.3) {
      const grid = proposeGrid(300, avail, cell, 2);
      if (!grid) continue;
      // Rendered height in CSS px must fit the content box (±0.5px canvas
      // rounding, absorbed by the host's bottom gutter).
      expect((grid.rows * cell.height) / 2).toBeLessThanOrEqual(avail + 0.5);
    }
  });

  it("clamps to the 2x1 minimum grid", () => {
    expect(proposeGrid(1, 1, cell, 2)).toEqual({ cols: 2, rows: 1 });
  });

  it("returns null when host or renderer is not measurable", () => {
    expect(proposeGrid(0, 300, cell, 2)).toBeNull();
    expect(proposeGrid(300, -10, cell, 2)).toBeNull();
    expect(proposeGrid(300, 300, { width: 0, height: 0 }, 2)).toBeNull();
    expect(proposeGrid(300, 300, { width: NaN, height: 33 }, 2)).toBeNull();
  });
});

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
