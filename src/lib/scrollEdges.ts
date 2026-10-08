/**
 * Marks a scroll container with the edges that still have content beyond
 * them (`data-more-top` / `-bottom` / `-left` / `-right`), so CSS can fade
 * just those edges. A CSS-only background shadow would paint *under* the
 * children and vanish behind opaque cards, hence the attributes.
 */

const EDGES = ["Top", "Bottom", "Left", "Right"] as const;

export function markScrollEdges(el: HTMLElement) {
  const more = {
    Top: el.scrollTop > 1,
    Bottom: el.scrollTop + el.clientHeight < el.scrollHeight - 1,
    Left: el.scrollLeft > 1,
    Right: el.scrollLeft + el.clientWidth < el.scrollWidth - 1,
  };
  for (const edge of EDGES) {
    if (more[edge]) el.dataset[`more${edge}`] = "";
    else delete el.dataset[`more${edge}`];
  }
}

/**
 * Ref callback keeping `markScrollEdges` current on scroll, resize and content
 * changes. Module-level (stable identity) and returns its cleanup, React 19
 * style, so one function serves every scroller, including ones in a `.map`.
 */
export function scrollEdges(el: HTMLElement | null) {
  if (!el) return;
  const update = () => markScrollEdges(el);
  update();
  el.addEventListener("scroll", update, { passive: true });
  const resize =
    typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
  resize?.observe(el);
  const mutate = new MutationObserver(update);
  mutate.observe(el, { childList: true, subtree: true });
  return () => {
    el.removeEventListener("scroll", update);
    resize?.disconnect();
    mutate.disconnect();
  };
}

const DRAG_ZONE = 64;
const DRAG_MAX_STEP = 24;

/**
 * Scroll step for a drag pointer at `pos` inside a scroller spanning
 * `start`..`end` on one axis: negative near the start, positive near the end,
 * faster the closer to the edge, 0 in the middle.
 */
export function dragScrollStep(pos: number, start: number, end: number) {
  const zone = Math.min(DRAG_ZONE, (end - start) / 3);
  const depth =
    pos < start + zone
      ? -(start + zone - pos)
      : pos > end - zone
        ? pos - (end - zone)
        : 0;
  const step = Math.min(Math.abs(depth) / zone, 1) * DRAG_MAX_STEP;
  return Math.sign(depth) * Math.ceil(step);
}

/**
 * `onDragOver` handler scrolling `el` while a drag hovers near its edges.
 * WebKit (Tauri's macOS webview) does not auto-scroll overflow containers
 * during a native drag, and wheel scrolling is swallowed by the drag session.
 * dragover keeps firing while the pointer rests, so resting at an edge keeps
 * scrolling.
 */
export function dragAutoScroll(
  e: { clientX: number; clientY: number; currentTarget: HTMLElement },
  axis: "x" | "y",
) {
  const el = e.currentTarget;
  const r = el.getBoundingClientRect();
  if (axis === "x") el.scrollLeft += dragScrollStep(e.clientX, r.left, r.right);
  else el.scrollTop += dragScrollStep(e.clientY, r.top, r.bottom);
}
