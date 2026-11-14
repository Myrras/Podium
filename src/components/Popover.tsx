/** A popover anchored under an element: portalled, follows it, no backdrop. */

import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";
import { createPortal } from "react-dom";

import type { Rect, Size } from "../lib/floor/types";
import { MOTION } from "../lib/motion";
import { clampToViewport, type Follow, MARGIN } from "../lib/viewport";
import { CaretIcon, CloseIcon } from "./icons";
import styles from "./Popover.module.css";

const GAP = 8;

type Slots = { header: HTMLElement | null; footer: HTMLElement | null };
const SlotContext = createContext<(Slots & { follow?: Follow }) | null>(null);

/** Whether the caller sits inside a Popover (its header/footer slots exist). */
export const useInPopover = () => useContext(SlotContext) !== null;

/** The enclosing Popover's `follow`, so a right-click menu inside it closes when it moves. */
export const usePopoverFollow = () => useContext(SlotContext)?.follow;

/**
 * Renders children into the enclosing Popover's header (left of ×) or footer,
 * so a child keeps its own state while its controls sit in the chrome.
 * Outside a Popover the children render in place.
 */
export function PopoverSlot({
  name,
  children,
}: {
  name: keyof Slots;
  children: ReactNode;
}) {
  const slots = useContext(SlotContext);
  if (!slots) return <>{children}</>;
  const el = slots[name];
  return el ? createPortal(children, el) : null;
}

/**
 * Blurs the focused element when it sits inside `el`, so a field's onBlur
 * commit runs before a close unmounts it. React delivers no blur for an
 * element its commit removes, so callers blur first, then close.
 */
export function blurWithin(el: Element | null | undefined): void {
  const at = document.activeElement;
  if (at instanceof HTMLElement && el?.contains(at)) at.blur();
}

/** Centred, GAP under the anchor; above it when below does not fit; MARGIN inside the window. */
export function placePopover(
  anchor: Rect,
  size: Size,
  viewport: Size,
): { left: number; top: number; above: boolean } {
  const left = clampToViewport(
    anchor.x + anchor.w / 2 - size.w / 2,
    size.w,
    viewport.w,
  );
  const below = anchor.y + anchor.h + GAP;
  const above = below + size.h + MARGIN > viewport.h;
  const top = above ? Math.max(MARGIN, anchor.y - GAP - size.h) : below;
  return { left, top, above };
}

export function Popover({
  anchor,
  title,
  count,
  onClose,
  onBack,
  backLabel,
  width = 360,
  flush = false,
  follow,
  children,
}: {
  anchor: HTMLElement | null;
  /** Omit when a child fills the header slot (an opened item's own title). */
  title?: ReactNode;
  /** Shown muted after the title. */
  count?: number;
  onClose: () => void;
  onBack?: () => void;
  backLabel?: string;
  width?: number;
  /** No body padding: the content brings its own (full-width rows). */
  flush?: boolean;
  /** What the anchor moves with (the Floor camera); without it, only resizes re-place. */
  follow?: Follow;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [header, setHeader] = useState<HTMLElement | null>(null);
  const [footer, setFooter] = useState<HTMLElement | null>(null);
  // Every close blurs a focused field first, so its own onBlur commit runs.
  const close = () => {
    blurWithin(ref.current);
    onClose();
  };
  const closeRef = useRef(close);
  closeRef.current = close;

  // Placed on open, on window resize, when the anchor or popover resizes, and
  // after each `follow` move; an animated one (a camera jump) is followed frame
  // by frame for its --dur-4 transition (MOTION.slow), then the loop stops.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!anchor || !el) return;
    let last = "";
    let raf = 0;
    let until = 0;
    const place = () => {
      // Its fixture unmounted (e.g. the last service removed): close, do not jump to 0,0.
      if (!anchor.isConnected) return closeRef.current();
      const r = anchor.getBoundingClientRect();
      const key = `${r.x},${r.y},${r.width},${r.height},${el.offsetWidth},${el.offsetHeight},${window.innerWidth},${window.innerHeight}`;
      if (key === last) return;
      last = key;
      const p = placePopover(
        { x: r.x, y: r.y, w: r.width, h: r.height },
        { w: el.offsetWidth, h: el.offsetHeight },
        { w: window.innerWidth, h: window.innerHeight },
      );
      el.style.left = `${p.left}px`;
      el.style.top = `${p.top}px`;
      el.dataset.above = String(p.above);
      el.style.visibility = "visible";
    };
    const tick = () => {
      raf = 0;
      place();
      if (performance.now() < until) raf = requestAnimationFrame(tick);
    };
    // At least one frame later, so FloorMap has written the layer transform first.
    const onMove = (animate: boolean) => {
      until = Math.max(until, performance.now() + (animate ? MOTION.slow : 0));
      if (!raf) raf = requestAnimationFrame(tick);
    };
    place();
    const off = follow?.(onMove);
    window.addEventListener("resize", place);
    // jsdom has no ResizeObserver.
    const ro =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(place);
    ro?.observe(anchor);
    ro?.observe(el);
    return () => {
      cancelAnimationFrame(raf);
      off?.();
      window.removeEventListener("resize", place);
      ro?.disconnect();
    };
  }, [anchor, follow]);

  // Focus in, so Esc works right after the click on the fixture; hand it back on close.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!anchor || !el) return;
    if (!el.contains(document.activeElement)) el.focus({ preventScroll: true });
    return () => {
      if (el.contains(document.activeElement) && anchor.isConnected)
        anchor.focus?.({ preventScroll: true });
    };
  }, [anchor]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      const t = e.target as Element;
      // Modals opened from inside a popover are portalled to body;
      // [data-popover-keep] marks map pan/zoom surfaces.
      if (
        ref.current?.contains(t) ||
        anchor?.contains(t) ||
        t.closest?.('[role="dialog"], [data-popover-keep]')
      )
        return;
      closeRef.current();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [anchor]);

  if (!anchor) return null;

  return createPortal(
    <div
      ref={ref}
      className={styles.popover}
      role="group"
      tabIndex={-1}
      style={{ width, visibility: "hidden" }}
      onKeyDown={(e) => {
        // Skip Esc an inline editor already used, or that bubbled from a portalled modal.
        if (
          e.key === "Escape" &&
          !e.defaultPrevented &&
          ref.current?.contains(e.target as Node)
        ) {
          e.stopPropagation();
          close();
        }
      }}
    >
      <header className={styles.header}>
        {onBack && (
          <button
            type="button"
            className={styles.back}
            title={backLabel ?? "Back"}
            onClick={onBack}
          >
            <CaretIcon />
            <span className="visually-hidden">{backLabel ?? "Back"}</span>
          </button>
        )}
        {title !== undefined && (
          <h2 className={styles.title}>
            {title}
            {count !== undefined && (
              <span className={styles.count}>{count}</span>
            )}
          </h2>
        )}
        <div ref={setHeader} className={styles.slot} />
        <button type="button" aria-label="Close" onClick={close}>
          <CloseIcon />
        </button>
      </header>
      <SlotContext.Provider value={{ header, footer, follow }}>
        <div className={styles.body} data-flush={flush || undefined}>
          {children}
        </div>
      </SlotContext.Provider>
      <footer ref={setFooter} className={styles.footer} />
    </div>,
    document.body,
  );
}
