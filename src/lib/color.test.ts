import { describe, expect, it } from "vitest";

import { readableOn, themeAccentHex } from "./color";

describe("readableOn", () => {
  it("picks black on light and white on dark", () => {
    expect(readableOn("#ffffff")).toBe("#000");
    expect(readableOn("#000000")).toBe("#fff");
  });
});

describe("themeAccentHex", () => {
  it("falls back to black without a hex accent token", () => {
    expect(themeAccentHex("dark")).toBe("#000000");
  });
});
