import { describe, expect, it } from "vitest";
import { botVariants } from "./botVariant";

const ids = (n: number) => Array.from({ length: n }, (_, i) => `agent-${i}`);

describe("botVariants", () => {
  it("gives 8 ids 8 different designs", () => {
    const m = botVariants(ids(8));
    expect(new Set(m.values()).size).toBe(8);
    for (const v of m.values()) expect(v).toBeGreaterThanOrEqual(0);
  });

  it("keeps earlier ids stable when another id joins later", () => {
    const before = botVariants(ids(5));
    const after = botVariants([...ids(5), "late-comer"]);
    for (const [id, v] of before) expect(after.get(id)).toBe(v);
    expect(new Set(after.values()).size).toBe(6);
  });

  it("still assigns a design past 8 ids", () => {
    expect(botVariants(ids(10)).size).toBe(10);
  });
});
