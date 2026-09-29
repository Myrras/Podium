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
