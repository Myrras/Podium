import { describe, expect, it } from "vitest";

import { activityLabel } from "./processStatus";

describe("activityLabel", () => {
  it("labels the three activities", () => {
    expect(activityLabel("working")).toBe("working…");
    expect(activityLabel("waiting")).toBe("needs input");
    expect(activityLabel("idle")).toBe("idle");
  });

  it("qualifies the label with background shells", () => {
    expect(activityLabel("idle", 1)).toBe("idle · 1 shell");
    expect(activityLabel("idle", 2)).toBe("idle · 2 shells");
    expect(activityLabel("working", 3)).toBe("working… · 3 shells");
  });
});
