import { beforeEach, describe, expect, it, vi } from "vitest";

const KEY = "podium.layout";

/** A fresh layoutStore module that loads from the current localStorage. */
async function freshStore() {
  vi.resetModules();
  return (await import("./layoutStore")).useLayoutStore;
}

const saved = () => JSON.parse(localStorage.getItem(KEY) ?? "{}");

beforeEach(() => localStorage.clear());

describe("layoutStore — mode and floor positions", () => {
  it("defaults to no mode and no positions", async () => {
    const store = await freshStore();
    expect(store.getState().mode).toBeNull();
    expect(store.getState().floorPositions).toEqual({});
  });

  it("loads valid values and drops invalid ones per field", async () => {
    localStorage.setItem(
      KEY,
      JSON.stringify({
        mode: "floor",
        floorPositions: {
          "/work/alpha": [40, 80],
          "/work/beta": [1, "2"],
          "/work/gamma": [1],
          "/work/delta": null,
        },
      }),
    );
    let store = await freshStore();
    expect(store.getState().mode).toBe("floor");
    expect(store.getState().floorPositions).toEqual({
      "/work/alpha": [40, 80],
    });

    localStorage.setItem(
      KEY,
      JSON.stringify({ mode: "sideways", floorPositions: [1, 2] }),
    );
    store = await freshStore();
    expect(store.getState().mode).toBeNull();
    expect(store.getState().floorPositions).toEqual({});
  });

  it("persists mode and positions, and other writes keep them", async () => {
    const store = await freshStore();
    store.getState().setMode("floor");
    store.getState().setFloorPosition("/work/alpha", [120, 40]);
    store.getState().setSidebarWidth(300);
    expect(saved()).toMatchObject({
      mode: "floor",
      floorPositions: { "/work/alpha": [120, 40] },
      sidebarWidth: 300,
    });

    const reloaded = await freshStore();
    expect(reloaded.getState().mode).toBe("floor");
    expect(reloaded.getState().floorPositions).toEqual({
      "/work/alpha": [120, 40],
    });
  });

  it("setFloorPositions writes several positions at once and keeps others", async () => {
    const store = await freshStore();
    store.getState().setFloorPosition("/work/alpha", [0, 0]);
    // Swap the global: on jsdom's real Storage an instance spy misses writes.
    const real = localStorage;
    const write = vi.fn((k: string, v: string) => real.setItem(k, v));
    vi.stubGlobal("localStorage", { setItem: write });
    store.getState().setFloorPositions({
      "/work/beta": [400, 0],
      "/work/gamma": [0, 400],
    });
    expect(write).toHaveBeenCalledTimes(1);
    vi.unstubAllGlobals();
    expect(saved().floorPositions).toEqual({
      "/work/alpha": [0, 0],
      "/work/beta": [400, 0],
      "/work/gamma": [0, 400],
    });
  });

  it("forgetProject drops its position, resetFloorPositions drops all", async () => {
    const store = await freshStore();
    store.getState().setFloorPosition("/work/alpha", [0, 0]);
    store.getState().setFloorPosition("/work/beta", [400, 0]);
    store.getState().forgetProject("/work/alpha");
    expect(store.getState().floorPositions).toEqual({ "/work/beta": [400, 0] });
    expect(saved().floorPositions).toEqual({ "/work/beta": [400, 0] });

    store.getState().resetFloorPositions();
    expect(store.getState().floorPositions).toEqual({});
    expect(saved().floorPositions).toEqual({});
  });
});
