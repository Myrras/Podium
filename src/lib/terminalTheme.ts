/**
 * Builds an xterm.js `ITheme` from the `--term-*` design tokens in
 * `tokens.css`. Those tokens are literal hex values per theme (xterm cannot
 * parse `var()`/`color-mix()`), so reading the computed values of
 * `document.documentElement` always yields the active theme's palette.
 */

import type { ITheme } from "@xterm/xterm";

export function readTerminalTheme(): ITheme {
  const cs = getComputedStyle(document.documentElement);
  const v = (name: string) => cs.getPropertyValue(name).trim();
  return {
    background: v("--term-bg"),
    foreground: v("--term-fg"),
    cursor: v("--term-cursor"),
    selectionBackground: v("--term-selection"),
    black: v("--term-ansi-black"),
    red: v("--term-ansi-red"),
    green: v("--term-ansi-green"),
    yellow: v("--term-ansi-yellow"),
    blue: v("--term-ansi-blue"),
    magenta: v("--term-ansi-magenta"),
    cyan: v("--term-ansi-cyan"),
    white: v("--term-ansi-white"),
    brightBlack: v("--term-ansi-bright-black"),
    brightRed: v("--term-ansi-bright-red"),
    brightGreen: v("--term-ansi-bright-green"),
    brightYellow: v("--term-ansi-bright-yellow"),
    brightBlue: v("--term-ansi-bright-blue"),
    brightMagenta: v("--term-ansi-bright-magenta"),
    brightCyan: v("--term-ansi-bright-cyan"),
    brightWhite: v("--term-ansi-bright-white"),
  };
}

/** The terminal font stack (mirrors `--font-mono`). */
export function readTerminalFontFamily(): string {
  const value = getComputedStyle(document.documentElement)
    .getPropertyValue("--font-mono")
    .trim();
  return value || "monospace";
}
