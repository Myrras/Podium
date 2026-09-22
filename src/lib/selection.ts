/**
 * Text the user has selected anywhere in Podium, tracked so the assistant
 * chat can offer it as context for the next message.
 *
 * Two sources feed it: the webview's own selection (detail panes, the chat,
 * markdown) via `selectionchange`, and xterm terminals — those render to a
 * canvas, so their selection never reaches the DOM and the registry pushes
 * it in by hand.
 *
 * Deliberately store-only (no imports from `src/state`): the terminal
 * registry feeds it, and `assistantContext` reads it, so anything richer
 * here would close a cycle between the two.
 */

import { create } from "zustand";

/** Longer than the backend would inline as a text attachment anyway. */
const MAX_SELECTION_CHARS = 100_000;

interface SelectionState {
  /** The current selection, trimmed; `""` when nothing is selected. */
  text: string;
}

export const useSelectionStore = create<SelectionState>(() => ({ text: "" }));

/** Replace the tracked selection (blank or oversized clears it). */
export function setSelectedText(text: string): void {
  const trimmed = text.trim();
  const next = trimmed.length > MAX_SELECTION_CHARS ? "" : trimmed;
  if (useSelectionStore.getState().text !== next) {
    useSelectionStore.setState({ text: next });
  }
}

/** Is this node inside the assistant panel? */
function inAssistantPane(node: Node | null): boolean {
  const el = node instanceof Element ? node : (node?.parentElement ?? null);
  return !!el?.closest("[data-assistant-pane]");
}

if (typeof document !== "undefined") {
  document.addEventListener("selectionchange", () => {
    const selection = document.getSelection();
    // Selecting *inside* the assistant is reading or editing the
    // conversation, not picking context for it — and clicking into the
    // composer collapses the document selection, which would otherwise throw
    // away the very text the user is about to ask about.
    if (
      inAssistantPane(selection?.anchorNode ?? null) ||
      inAssistantPane(document.activeElement)
    ) {
      return;
    }
    setSelectedText(selection?.toString() ?? "");
  });
}
