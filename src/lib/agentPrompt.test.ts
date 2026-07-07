import { describe, expect, it } from "vitest";

import { detectInputPrompt } from "./agentPrompt";

describe("detectInputPrompt", () => {
  it("flags a Claude Code permission menu", () => {
    const screen = [
      "  Edit file src/main.rs",
      "",
      "Do you want to proceed?",
      "❯ 1. Yes",
      "  2. Yes, and don't ask again this session",
      "  3. No, and tell Claude what to do differently",
    ].join("\n");
    expect(detectInputPrompt(screen)).toBe(true);
  });

  it("flags a numbered selection pointer", () => {
    expect(detectInputPrompt("Pick one:\n❯ 2. Rebase")).toBe(true);
  });

  it("flags classic yes/no confirmations", () => {
    expect(detectInputPrompt("Overwrite existing file? (y/n)")).toBe(true);
    expect(detectInputPrompt("Continue? [y/N]")).toBe(true);
  });

  it("flags explicit input/permission requests", () => {
    expect(detectInputPrompt("Press Enter to continue")).toBe(true);
    expect(detectInputPrompt("Waiting for your approval…")).toBe(true);
    expect(detectInputPrompt("Permission required to run this command")).toBe(
      true,
    );
  });

  it("ignores ordinary agent output", () => {
    const screen = [
      "Running tests…",
      "  ok src/lib/foo.rs",
      "Compiled successfully in 1.2s",
      "Wrote 3 files.",
    ].join("\n");
    expect(detectInputPrompt(screen)).toBe(false);
  });

  it("is false for empty text", () => {
    expect(detectInputPrompt("")).toBe(false);
  });

  it("only considers the tail of a long screen", () => {
    const prompt = "Do you want to proceed?";
    const noise = "x".repeat(5000);
    // Prompt scrolled far off the top (beyond the 4000-char tail) is ignored.
    expect(detectInputPrompt(prompt + noise)).toBe(false);
    // Prompt at the bottom is caught.
    expect(detectInputPrompt(noise + prompt)).toBe(true);
  });
});
