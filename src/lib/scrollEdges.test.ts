import { describe, expect, it } from "vitest";

import { dragScrollStep, markScrollEdges } from "./scrollEdges";

function scroller(box: Partial<Record<string, number>>) {
  const el = document.createElement("div");
  for (const [k, v] of Object.entries(box)) {
    Object.defineProperty(el, k, { value: v, configurable: true });
  }
  return el;
}

describe("markScrollEdges", () => {
  it("marks only the edges with more content", () => {
    const el = scroller({
      scrollTop: 50,
      clientHeight: 100,
      scrollHeight: 300,
      scrollLeft: 0,
      clientWidth: 200,
      scrollWidth: 200,
    });
    markScrollEdges(el);
    expect(el.hasAttribute("data-more-top")).toBe(true);
    expect(el.hasAttribute("data-more-bottom")).toBe(true);
    expect(el.hasAttribute("data-more-left")).toBe(false);
    expect(el.hasAttribute("data-more-right")).toBe(false);
  });

  it("clears a mark once the end is reached", () => {
    const el = scroller({ scrollTop: 0, clientHeight: 100, scrollHeight: 300 });
    markScrollEdges(el);
    expect(el.hasAttribute("data-more-bottom")).toBe(true);
    Object.defineProperty(el, "scrollTop", { value: 200 });
    markScrollEdges(el);
    expect(el.hasAttribute("data-more-bottom")).toBe(false);
    expect(el.hasAttribute("data-more-top")).toBe(true);
  });
});

describe("dragScrollStep", () => {
  it("is 0 in the middle and grows towards each edge", () => {
    expect(dragScrollStep(500, 0, 1000)).toBe(0);
    expect(dragScrollStep(60, 0, 1000)).toBeLessThan(0);
    expect(dragScrollStep(0, 0, 1000)).toBe(-24);
    expect(dragScrollStep(1000, 0, 1000)).toBe(24);
    expect(dragScrollStep(990, 0, 1000)).toBeGreaterThan(
      dragScrollStep(950, 0, 1000),
    );
  });

  it("caps the step past the edge and shrinks the zone for small scrollers", () => {
    expect(dragScrollStep(-50, 0, 1000)).toBe(-24);
    expect(dragScrollStep(50, 0, 90)).toBe(0);
  });
});
