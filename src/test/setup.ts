/**
 * Vitest setup shared by all jsdom test files.
 *
 * - Registers `@testing-library/jest-dom` matchers (`toBeInTheDocument`, ...).
 * - Unmounts React trees and clears the document body after every test so
 *   component tests stay isolated (RTL's auto-cleanup is not assumed).
 * - Ensures bare `localStorage` references use a complete in-memory `Storage`.
 *   Some Node/Vitest launches expose an incomplete Node global instead, which
 *   lacks methods like `clear()` and breaks storage-focused unit tests.
 * - Shims `Range.prototype.getClientRects`/`getBoundingClientRect`, which
 *   jsdom doesn't implement. ProseMirror's `EditorView.scrollToSelection`
 *   (invoked by Tiptap's `focus()`/`scrollIntoView()` commands) calls
 *   `coordsAtPos`, which needs these on a `Range` — without the shim it
 *   throws an uncaught `TypeError` from inside prosemirror-view on every test
 *   that focuses a real Tiptap editor (see `ScratchpadEditor.test.tsx`).
 */

import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => Array.from(values.keys())[index] ?? null,
    removeItem: (key) => values.delete(key),
    setItem: (key, value) => values.set(key, String(value)),
  };
}

if (
  typeof globalThis.localStorage?.clear !== "function" ||
  typeof globalThis.localStorage?.setItem !== "function"
) {
  const storage = memoryStorage();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: storage,
  });
  if (typeof window !== "undefined") {
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      value: storage,
    });
  }
}


// jsdom has no layout engine, so `Range`/`Element` never report real
// geometry — stub them out to zero-sized rects so ProseMirror's
// `coordsAtPos`/`scrollToSelection` (used by Tiptap's `focus()` and content
// commands) have something to call instead of throwing.
if (typeof Range !== "undefined" && !Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () => ({
    length: 0,
    item: () => null,
    [Symbol.iterator]: function* () {},
  }) as unknown as DOMRectList;
}
if (typeof Range !== "undefined" && !Range.prototype.getBoundingClientRect) {
  Range.prototype.getBoundingClientRect = () =>
    ({
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      top: 0,
      left: 0,
      right: 0,
      bottom: 0,
      toJSON: () => ({}),
    }) as DOMRect;
}

afterEach(() => {
  cleanup();
});
