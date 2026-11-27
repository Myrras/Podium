import type { ThemeMode } from "../state/themeStore";

/** Resolved `--accent` per theme; the CSS lookup is not free. */
const accentByTheme = new Map<ThemeMode, string>();

/**
 * The active theme's `--accent` as `#rrggbb`, what a colour input takes.
 * Read from the CSS tokens (once per theme) rather than duplicated here.
 */
export function themeAccentHex(mode: ThemeMode): string {
  const cached = accentByTheme.get(mode);
  if (cached) return cached;
  const accent = getComputedStyle(document.documentElement)
    .getPropertyValue("--accent")
    .trim()
    .toLowerCase();
  const hex = /^#[0-9a-f]{6}$/.test(accent) ? accent : "#000000";
  accentByTheme.set(mode, hex);
  return hex;
}

/** Black or white initials, whichever reads better on a `#rrggbb` badge. */
export function readableOn(hex: string): string {
  const n = parseInt(hex.slice(1), 16);
  // WCAG relative luminance; pick whichever of black/white contrasts more.
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  const l =
    0.2126 * lin(n >> 16) +
    0.7152 * lin((n >> 8) & 255) +
    0.0722 * lin(n & 255);
  return (l + 0.05) / 0.05 > 1.05 / (l + 0.05) ? "#000" : "#fff";
}
