import { describe, expect, it } from "vitest";

import { parseUnifiedDiff } from "./diff";

const SAMPLE = [
  "diff --git a/src/a.ts b/src/a.ts",
  "index 1111111..2222222 100644",
  "--- a/src/a.ts",
  "+++ b/src/a.ts",
  "@@ -3,3 +3,4 @@ export function demo() {",
  " keep",
  "-old line",
  "+new line",
  "+extra line",
  " tail",
  "\\ No newline at end of file",
  "",
].join("\n");

describe("parseUnifiedDiff", () => {
  it("drops the file header and numbers each line", () => {
    const rows = parseUnifiedDiff(SAMPLE);
    expect(rows.map((r) => r.kind)).toEqual([
      "hunk",
      "context",
      "del",
      "add",
      "add",
      "context",
      "meta",
    ]);
    expect(rows[1]).toMatchObject({ text: "keep", oldLine: 3, newLine: 3 });
    expect(rows[2]).toMatchObject({ text: "old line", oldLine: 4 });
    expect(rows[3]).toMatchObject({ text: "new line", newLine: 4 });
    expect(rows[4]).toMatchObject({ text: "extra line", newLine: 5 });
    expect(rows[5]).toMatchObject({ text: "tail", oldLine: 5, newLine: 6 });
  });

  it("strips CRLF endings", () => {
    const rows = parseUnifiedDiff("@@ -1 +1 @@\r\n-a\r\n+b\r\n");
    expect(rows[1].text).toBe("a");
    expect(rows[2].text).toBe("b");
  });

  it("returns nothing for an empty diff", () => {
    expect(parseUnifiedDiff("")).toEqual([]);
  });
});
