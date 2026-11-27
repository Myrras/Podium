/** User-resized modal sizes, remembered per kind (not per project) in localStorage. */

import { useCallback } from "react";

import type { Size } from "./floor/types";
import { MARGIN } from "./viewport";

const storageKey = (key: string) => `podium.size.${key}`;

/** The size saved under `key`; null when there is none or it does not fit the window. */
export function loadSize(key: string): Size | null {
  try {
    const s = JSON.parse(
      localStorage.getItem(storageKey(key)) ?? "null",
    ) as Partial<Size> | null;
    if (typeof s?.w !== "number" || typeof s.h !== "number") return null;
    const fits =
      s.w <= window.innerWidth - 2 * MARGIN &&
      s.h <= window.innerHeight - 2 * MARGIN;
    return fits ? { w: s.w, h: s.h } : null;
  } catch {
    return null;
  }
}

export function saveSize(key: string, size: Size): void {
  try {
    localStorage.setItem(storageKey(key), JSON.stringify(size));
  } catch {
    /* quota exceeded */
  }
}

/**
 * A ref for an element the user resizes with CSS `resize: both`: on mount it
 * gets the size saved under `key` (when that fits the window) and each size it
 * is dragged to is saved. The element must not set an inline width or height
 * itself: one marks a size the user chose. `data-sized` is set while it has one.
 * Without `key` it does nothing. The returned cleanup (key change or unmount)
 * clears the inline size, so a reused element does not keep it.
 */
export function useSavedSize(key: string | undefined) {
  return useCallback(
    (el: HTMLElement | null) => {
      if (!key || !el) return;
      const saved = loadSize(key);
      if (saved) {
        el.style.width = `${saved.w}px`;
        el.style.height = `${saved.h}px`;
        el.dataset.sized = "";
      }
      // jsdom has no ResizeObserver.
      const ro =
        typeof ResizeObserver === "undefined"
          ? null
          : new ResizeObserver(() => {
              // Content and window changes leave the inline size alone; a drag sets it.
              if (!el.style.width && !el.style.height) return;
              el.dataset.sized = "";
              saveSize(key, { w: el.offsetWidth, h: el.offsetHeight });
            });
      ro?.observe(el);
      return () => {
        ro?.disconnect();
        el.style.removeProperty("width");
        el.style.removeProperty("height");
        delete el.dataset.sized;
      };
    },
    [key],
  );
}
