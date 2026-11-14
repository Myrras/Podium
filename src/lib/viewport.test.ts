import { describe, expect, it } from "vitest";

import { clampToViewport } from "./viewport";

describe("clampToViewport", () => {
  it("keeps the value when it fits", () => {
    expect(clampToViewport(100, 200, 1000)).toBe(100);
  });
  it("stays 6px inside both edges", () => {
    expect(clampToViewport(-5, 200, 1000)).toBe(6);
    expect(clampToViewport(990, 200, 1000)).toBe(794);
  });
  it("keeps the start edge when the box does not fit", () => {
    expect(clampToViewport(50, 1200, 1000)).toBe(6);
  });
});
