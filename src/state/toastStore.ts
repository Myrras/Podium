/**
 * Non-blocking toast notifications (errors, progress, success).
 *
 * Toasts never carry secrets — callers pass already-sanitized text.
 */

import { create } from "zustand";

import { logErr } from "../lib/log";
import { MOTION, prefersReducedMotion } from "../lib/motion";

export type ToastKind = "error" | "success" | "info";

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
  /** Optional detail line (e.g. an error kind). */
  detail?: string;
  /** When set, the toast is "sticky" and updated in place (progress). */
  sticky?: boolean;
  /**
   * Set while the toast plays its exit animation, just before it is removed.
   * The view keys its `data-state` off this so the card can slide/fade out.
   */
  leaving?: boolean;
}

interface ToastState {
  toasts: Toast[];
  push: (toast: Omit<Toast, "id">) => number;
  update: (id: number, patch: Partial<Omit<Toast, "id">>) => void;
  /** Animate the toast out, then remove it. Use this for user/auto dismissal. */
  requestDismiss: (id: number) => void;
  /** Remove a toast immediately (no exit animation). */
  dismiss: (id: number) => void;
}

let seq = 0;
const AUTO_DISMISS_MS = 5000;

export const useToastStore = create<ToastState>((set, get) => ({
  toasts: [],
  push: (toast) => {
    seq += 1;
    const id = seq;
    set((state) => ({ toasts: [...state.toasts, { ...toast, id }] }));
    if (!toast.sticky) {
      // Auto-dismiss goes through the animated path so it eases out too.
      setTimeout(() => get().requestDismiss(id), AUTO_DISMISS_MS);
    }
    return id;
  },
  update: (id, patch) =>
    set((state) => ({
      toasts: state.toasts.map((t) => (t.id === id ? { ...t, ...patch } : t)),
    })),
  requestDismiss: (id) => {
    const existing = get().toasts.find((t) => t.id === id);
    if (!existing || existing.leaving) return; // already gone or leaving
    set((state) => ({
      toasts: state.toasts.map((t) =>
        t.id === id ? { ...t, leaving: true } : t,
      ),
    }));
    const delay = prefersReducedMotion() ? 0 : MOTION.base;
    setTimeout(() => get().dismiss(id), delay);
  },
  dismiss: (id) =>
    set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) })),
}));

/** Convenience for the common error path. Also writes a sanitized log line. */
export function toastError(message: string, detail?: string): number {
  // `message`/`detail` are caller-sanitized (UI text) — no secrets ever reach
  // here.
  logErr(message, detail ?? "");
  return useToastStore.getState().push({ kind: "error", message, detail });
}

export function toastSuccess(message: string): number {
  return useToastStore.getState().push({ kind: "success", message });
}

export function toastInfo(message: string, detail?: string): number {
  return useToastStore.getState().push({ kind: "info", message, detail });
}
