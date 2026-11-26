import type { KeyboardEvent } from "react";

const FOCUSABLE =
  'button:not([disabled]), input, textarea, [href], [tabindex]:not([tabindex="-1"])';

// Tab stays inside the element it is bound to. A focused terminal never gets
// here: xterm cancels Tab (and Esc) itself and sends it to the PTY.
export function trapTab(e: KeyboardEvent<HTMLDivElement>) {
  if (e.key !== "Tab" || e.defaultPrevented) return;
  const items = Array.from(
    e.currentTarget.querySelectorAll<HTMLElement>(FOCUSABLE),
  );
  if (items.length === 0) return;
  const first = items[0];
  const last = items[items.length - 1];
  const at = document.activeElement;
  if (e.shiftKey && (at === first || !e.currentTarget.contains(at))) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && (at === last || !e.currentTarget.contains(at))) {
    e.preventDefault();
    first.focus();
  }
}
