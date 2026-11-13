import { describe, expect, it } from "vitest";
import {
  botSheet,
  miniSheet,
  monitorSheet,
  shade,
  terminalDeskSheet,
} from "./sprites";

const svg = (v: string) =>
  decodeURIComponent(v.slice('url("data:image/svg+xml,'.length, -2));

describe("sprites", () => {
  it("returns css url values holding svg", () => {
    for (const v of [
      botSheet("#58a6ff"),
      miniSheet("#58a6ff"),
      monitorSheet(),
      terminalDeskSheet(),
    ]) {
      expect(v).toMatch(/^url\("data:image\/svg\+xml,/);
      expect(svg(v)).toContain("crispEdges");
    }
  });
  it("sizes sheets by frame count", () => {
    expect(svg(botSheet("#58a6ff"))).toContain('width="128" height="16"');
    expect(svg(miniSheet("#58a6ff"))).toContain('width="16" height="8"');
    expect(svg(monitorSheet())).toContain('width="16" height="8"');
  });
  it("uses the body colour and its shades, and caches", () => {
    const v = botSheet("#3fb950");
    expect(svg(v)).toContain('fill="#3fb950"');
    expect(svg(v)).toContain(shade("#3fb950", 0.72));
    expect(botSheet("#3fb950")).toBe(v);
  });
});
