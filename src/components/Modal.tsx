/** A small, accessible modal dialog (overlay + centered card). */

import { useCallback, useEffect, useRef } from "react";
import type { CSSProperties, ReactNode } from "react";
import { createPortal } from "react-dom";

import { MOTION, usePresence } from "../lib/motion";
import { useSavedSize } from "../lib/useSavedSize";
import { CloseIcon } from "./icons";
import styles from "./Modal.module.css";

interface ModalProps {
  open: boolean;
  title: string;
  onClose: () => void;
  children: ReactNode;
  /** Footer actions (buttons), rendered right-aligned. */
  footer?: ReactNode;
  /** Tone of the header accent bar. */
  tone?: "default" | "danger" | "warning";
  /** Card width — a px number or any CSS width (e.g. `"min(960px, 94vw)"`). */
  width?: number | string;
  /** Makes it user-resizable; the last size is kept under this key (`useSavedSize`). */
  sizeKey?: string;
}

export function Modal({
  open,
  title,
  onClose,
  children,
  footer,
  tone = "default",
  width = 460,
  sizeKey,
}: ModalProps) {
  const cardRef = useRef<HTMLDivElement>(null);
  const sized = useSavedSize(sizeKey);
  const setRef = useCallback(
    (el: HTMLDivElement | null) => {
      cardRef.current = el;
      const off = sized(el);
      return () => {
        cardRef.current = null;
        off?.();
      };
    },
    [sized],
  );
  // Keep the dialog mounted while it animates closed (see `usePresence`).
  const { mounted, state } = usePresence(open, MOTION.base);

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  // Focus the card only when the dialog *opens* — keyed on `open` alone so an
  // unstable `onClose` (a fresh closure each render of the parent) can't re-fire
  // this and steal focus from an input mid-typing.
  useEffect(() => {
    if (open) cardRef.current?.focus({ preventScroll: true });
  }, [open]);

  if (!mounted) return null;

  // Portal to <body>: a caller inside a transformed/overflow-clipped ancestor
  // (a Git board column) would otherwise clip or re-anchor `position: fixed`.
  return createPortal(
    <div
      className={styles.overlay}
      data-state={state}
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={setRef}
        className={styles.card}
        data-state={state}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        data-resizable={sizeKey !== undefined || undefined}
        // A variable, not `width`: an inline size is one the user dragged (useSavedSize).
        style={
          {
            "--modal-w": typeof width === "number" ? `${width}px` : width,
          } as CSSProperties
        }
      >
        <header className={`${styles.header} ${styles[tone]}`}>
          <h2 className={styles.title}>{title}</h2>
          <button
            type="button"
            className="ghost"
            aria-label="Close dialog"
            onClick={onClose}
          >
            <CloseIcon />
          </button>
        </header>
        <div className={styles.body}>{children}</div>
        {footer && <footer className={styles.footer}>{footer}</footer>}
      </div>
    </div>,
    document.body,
  );
}
