import { beforeEach, describe, expect, it, vi } from "vitest";

// The store reads the stored theme once, at module load, so each case imports
// a fresh copy.
async function loadStore() {
  vi.resetModules();
  return (await import("./themeStore")).useThemeStore;
}

describe("themeStore", () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute("data-theme");
  });

  it.each(["light", "retro", "glass-dark", "glass-light"])(
    "restores a stored %s theme",
    async (mode) => {
      localStorage.setItem("podium.theme", mode);
      const store = await loadStore();
      expect(store.getState().mode).toBe(mode);
      expect(document.documentElement.getAttribute("data-theme")).toBe(mode);
    },
  );

  it("falls back to dark for an unknown stored value", async () => {
    localStorage.setItem("podium.theme", "glass");
    const store = await loadStore();
    expect(store.getState().mode).toBe("dark");
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });

  it("persists and applies a new theme", async () => {
    const store = await loadStore();
    store.getState().setTheme("glass-light");
    expect(localStorage.getItem("podium.theme")).toBe("glass-light");
    expect(document.documentElement.getAttribute("data-theme")).toBe(
      "glass-light",
    );
    expect(store.getState().mode).toBe("glass-light");
  });
});
