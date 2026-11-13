import { afterEach, expect, it } from "vitest";

import { isGlassTheme } from "./terminalTheme";

afterEach(() => document.documentElement.removeAttribute("data-theme"));

it("detects glass themes only", () => {
  const set = (t: string) =>
    document.documentElement.setAttribute("data-theme", t);
  set("glass-dark");
  expect(isGlassTheme()).toBe(true);
  set("glass-light");
  expect(isGlassTheme()).toBe(true);
  set("dark");
  expect(isGlassTheme()).toBe(false);
});
