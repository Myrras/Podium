import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { loadSize, useSavedSize } from "./useSavedSize";

describe("loadSize", () => {
  beforeEach(() => localStorage.removeItem("podium.size.k"));

  it.each([
    ["missing", null],
    ["malformed JSON", "{nope"],
    ["non-number fields", '{"w":"500","h":400}'],
    ["too big for the window", '{"w":99999,"h":99999}'],
  ])("returns null when %s", (_name, raw) => {
    if (raw !== null) localStorage.setItem("podium.size.k", raw);
    expect(loadSize("k")).toBeNull();
  });

  it("returns a size that fits", () => {
    localStorage.setItem("podium.size.k", '{"w":300,"h":200}');
    expect(loadSize("k")).toEqual({ w: 300, h: 200 });
  });
});

describe("useSavedSize", () => {
  it("cleanup clears the inline size", () => {
    localStorage.setItem("podium.size.k", '{"w":300,"h":200}');
    const { result } = renderHook(() => useSavedSize("k"));
    const el = document.createElement("div");
    const cleanup = result.current(el) as () => void;
    expect(el.style.width).toBe("300px");
    cleanup();
    expect(el.style.width).toBe("");
    expect(el.style.height).toBe("");
    expect(el).not.toHaveAttribute("data-sized");
  });
});
