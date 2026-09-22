import { describe, expect, it } from "vitest";

import { textPreview } from "./textPreview";

describe("textPreview", () => {
  it("leaves short single-line text untouched", () => {
    expect(textPreview("Fix the login button")).toBe("Fix the login button");
  });

  it("collapses newlines and runs of whitespace to single spaces", () => {
    expect(textPreview("Fix the\n\n  login\tbutton\n")).toBe(
      "Fix the login button",
    );
  });

  it("truncates past the limit at a word boundary", () => {
    const text =
      "Tooltips for to-dos show the entire description dumped in, which makes them unreadable";
    const preview = textPreview(text);
    expect(preview.length).toBeLessThanOrEqual(81);
    expect(preview).toBe(
      "Tooltips for to-dos show the entire description dumped in, which makes them…",
    );
  });

  it("cuts mid-word rather than losing most of the budget", () => {
    const preview = textPreview(`${"a".repeat(40)} ${"b".repeat(40)}`, 20);
    expect(preview).toBe(`${"a".repeat(20)}…`);
  });

  it("honours a custom limit", () => {
    expect(textPreview("one two three four", 8)).toBe("one two…");
  });
});
