/** A popover anchored under an element: portalled, follows it, no backdrop. */

import { useEffect, useLayoutEffect, useRef } from "react";
import type { ReactNode } from "react";
import { createPortal } from "react-dom";

import type { Rect, Size } from "../lib/floor/types";
import { CloseIcon } from "./icons";
import styles from "./Popover.module.css";

const GAP = 8;
const MARGIN = 6;

/** Centred, GAP under the anchor; above it when below does not fit; MARGIN inside the window. */
export function placePopover(
  anchor: Rect,
  size: Size,
  viewport: Size,
): { left: number; top: number; above: boolean } {
  const clamp = (v: number, max: number) => Math.max(MARGIN, Math.min(v, max));
  const left = clamp(
    anchor.x + anchor.w / 2 - size.w / 2,
    viewport.w - size.w - MARGIN,
  );
  const below = anchor.y + anchor.h + GAP;
  const above = below + size.h + MARGIN > viewport.h;
  const top = above ? Math.max(MARGIN, anchor.y - GAP - size.h) : below;
  return { left, top, above };
}

export function Popover({
  anchor,
  title,
  onClose,
  onBack,
  backLabel,
  width = 360,
  children,
}: {
  anchor: HTMLElement | null;
  title: ReactNode;
  onClose: () => void;
  onBack?: () => void;
  backLabel?: string;
  width?: number;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  // shortcut: one rAF loop per open popover; upgrade to a shared loop when several can be open.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!anchor || !el) return;
    let last = "";
    let raf = 0;
    const update = () => {
      const r = anchor.getBoundingClientRect();
      const key = `${r.x},${r.y},${r.width},${r.height},${el.offsetWidth},${el.offsetHeight},${window.innerWidth},${window.innerHeight}`;
      if (key !== last) {
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
      }
      raf = requestAnimationFrame(update);
    };
    update();
    return () => cancelAnimationFrame(raf);
  }, [anchor]);

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
      // Modals opened from inside a popover are portalled to body.
      if (
        ref.current?.contains(t) ||
        anchor?.contains(t) ||
        t.closest?.('[role="dialog"]')
      )
        return;
      onCloseRef.current();
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
        if (e.key === "Escape") {
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <header className={styles.header}>
        {onBack && (
          <button type="button" className="ghost" onClick={onBack}>
            {backLabel ?? "Back"}
          </button>
        )}
        <h2 className={styles.title}>{title}</h2>
        <button
          type="button"
          className="ghost"
          aria-label="Close"
          onClick={onClose}
        >
          <CloseIcon />
        </button>
      </header>
      <div className={styles.body}>{children}</div>
    </div>,
    document.body,
  );
}
