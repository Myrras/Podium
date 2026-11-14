/** A right-click menu: portalled at a point, no backdrop. Closes on Esc, an
 *  outside mousedown or wheel, a window resize, and each `follow` move (the
 *  Floor camera), so it never stays at a stale spot.
 *  An item with `confirm` swaps the panel for a compact confirm view first. */

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { createPortal } from "react-dom";

import { clampToViewport, type Follow } from "../lib/viewport";
import styles from "./ContextMenu.module.css";

export type MenuConfirm = { title: string; body: string; action: string };

export type MenuItem =
  | {
      label: string;
      onSelect: () => void;
      danger?: boolean;
      disabled?: boolean;
      confirm?: MenuConfirm;
    }
  | "separator";

type Choice = Exclude<MenuItem, "separator">;

export function ContextMenu({
  x,
  y,
  items,
  onClose,
  follow,
}: {
  x: number;
  y: number;
  items: MenuItem[];
  onClose: () => void;
  /** What the point moves with (the Floor camera): the menu closes on each move. */
  follow?: Follow;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const [pending, setPending] = useState<Choice | null>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  const buttons = () =>
    Array.from(
      ref.current?.querySelectorAll<HTMLButtonElement>(
        'button[role="menuitem"]:not(:disabled)',
      ) ?? [],
    );

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.left = `${clampToViewport(x, el.offsetWidth, window.innerWidth)}px`;
    el.style.top = `${clampToViewport(y, el.offsetHeight, window.innerHeight)}px`;
    (pending ? cancelRef.current : buttons()[0])?.focus({
      preventScroll: true,
    });
  }, [x, y, pending]);

  useEffect(() => {
    const close = () => onCloseRef.current();
    const outside = (e: Event) => {
      if (!ref.current?.contains(e.target as Node)) close();
    };
    document.addEventListener("mousedown", outside);
    // Capture: the floor map's own wheel handler may stop it bubbling.
    window.addEventListener("wheel", outside, { capture: true, passive: true });
    window.addEventListener("resize", close);
    const off = follow?.(close);
    return () => {
      document.removeEventListener("mousedown", outside);
      window.removeEventListener("wheel", outside, { capture: true });
      window.removeEventListener("resize", close);
      off?.();
    };
  }, [follow]);

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      if (pending) setPending(null);
      else onClose();
      return;
    }
    if (pending) return;
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const list = buttons();
    if (!list.length) return;
    const i = list.indexOf(document.activeElement as HTMLButtonElement);
    const step = e.key === "ArrowDown" ? 1 : -1;
    list[(i + step + list.length) % list.length].focus();
  };

  return createPortal(
    <div
      ref={ref}
      className={styles.menu}
      role={pending ? "alertdialog" : "menu"}
      aria-label={pending?.confirm?.title}
      data-popover-keep
      style={{ left: x, top: y }}
      onKeyDown={onKeyDown}
      onContextMenu={(e) => e.preventDefault()}
    >
      {pending?.confirm ? (
        <div className={styles.confirm}>
          <div className={styles.confirmTitle}>{pending.confirm.title}</div>
          <div className={styles.confirmBody}>{pending.confirm.body}</div>
          <div className={styles.confirmActions}>
            <button
              ref={cancelRef}
              type="button"
              onClick={() => setPending(null)}
            >
              Cancel
            </button>
            <button
              type="button"
              className="danger"
              onClick={() => {
                onClose();
                pending.onSelect();
              }}
            >
              {pending.confirm.action}
            </button>
          </div>
        </div>
      ) : (
        items.map((item, i) =>
          item === "separator" ? (
            <div key={i} role="separator" className={styles.separator} />
          ) : (
            <button
              key={i}
              type="button"
              role="menuitem"
              disabled={item.disabled}
              className={`${styles.item} ${item.danger ? styles.danger : ""}`}
              onClick={() => {
                if (item.confirm) {
                  setPending(item);
                  return;
                }
                onClose();
                item.onSelect();
              }}
            >
              {item.label}
            </button>
          ),
        )
      )}
    </div>,
    document.body,
  );
}
