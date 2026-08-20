import { describe, expect, it } from "vitest";

import { detectBackgroundShells, detectInputPrompt } from "./agentPrompt";

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

  it("flags a sudo password prompt", () => {
    expect(detectInputPrompt("$ sudo just install-signed\nPassword:")).toBe(
      true,
    );
    expect(detectInputPrompt("[sudo] password for tester: \n")).toBe(true);
  });

  it("flags an ssh passphrase prompt", () => {
    expect(
      detectInputPrompt(
        "Enter passphrase for key '/home/tester/.ssh/id_ed25519':",
      ),
    ).toBe(true);
  });

  it("does not flag output that merely mentions a password", () => {
    expect(detectInputPrompt("warn: password rotation is due in 3 days")).toBe(
      false,
    );
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

describe("detectBackgroundShells", () => {
  // Verbatim shape of Claude Code's footer while background bash tasks run.
  const footer = (segment: string) =>
    `⏵⏵ auto mode on (shift+tab to cycle)${segment} · esc to interrupt · ← for agents · ↓ to manage`;

  it("reads the singular and plural counts off the footer", () => {
    expect(detectBackgroundShells(footer(" · 1 shell"))).toBe(1);
    expect(detectBackgroundShells(footer(" · 3 shells"))).toBe(3);
  });

  it("is 0 when the footer reports no shells", () => {
    expect(detectBackgroundShells(footer(""))).toBe(0);
    expect(detectBackgroundShells("")).toBe(0);
  });

  it("ignores a count that is not a footer segment", () => {
    // Ordinary output mentioning shells must not read as background work.
    expect(detectBackgroundShells("Rewrote 2 shells in src/shell.ts")).toBe(0);
    expect(detectBackgroundShells("shells: 4")).toBe(0);
    // Claude Code's own tool summary line is the nastiest look-alike.
    expect(detectBackgroundShells("· Ran 2 shell commands")).toBe(0);
    expect(detectBackgroundShells("· 2 shell commands ·")).toBe(0);
  });

  it("only considers the tail of a long screen", () => {
    const marker = " · 1 shell · ↓ to manage";
    const noise = "x".repeat(5000);
    expect(detectBackgroundShells(marker + noise)).toBe(0);
    expect(detectBackgroundShells(noise + marker)).toBe(1);
  });
});
