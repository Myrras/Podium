/** Viewport placement shared by `Popover` and `ContextMenu`. */

/** Distance kept from the window edges. */
export const MARGIN = 6;

/** One axis: `v` kept MARGIN inside a viewport of `viewport` for a box of `size` (the start edge wins). */
export function clampToViewport(
  v: number,
  size: number,
  viewport: number,
): number {
  return Math.max(MARGIN, Math.min(v, viewport - size - MARGIN));
}

/**
 * A subscription to moves of what a popover or menu sits on (e.g. the Floor
 * camera): calls `cb` on each move, with whether it animates; returns the
 * unsubscribe.
 */
export type Follow = (cb: (animate: boolean) => void) => () => void;
