/**
 * What the user has selected in Podium, tracked so the assistant chat can
 * offer it as context for the next message: sidebar items picked with
 * Cmd/Ctrl+click, and selected text.
 *
 * Items live here rather than in the subsections that render them because
 * two readers need them at once — the subsection (for its "start an agent on
 * these" bar) and the assistant composer (for its context chips). Merely
 * *opening* a to-do/scratchpad/process is deliberately not a selection: that
 * is a "show me this" gesture, and it would attach context nobody asked for.
 *
 * Two sources feed the *text* half: the webview's own selection (detail
 * panes, the chat, markdown) via `selectionchange`, and xterm terminals —
 * those render to a canvas, so their selection never reaches the DOM and the
 * registry pushes it in by hand.
 *
 * Deliberately store-only (no imports from `src/state`): the terminal
 * registry feeds it, and `assistantContext` reads it, so anything richer
 * here would close a cycle between the two.
 */

import { create } from "zustand";

/** Longer than the backend would inline as a text attachment anyway. */
const MAX_SELECTION_CHARS = 100_000;

/** A sidebar row the user has Cmd/Ctrl+clicked. */
export interface SelectedItem {
  kind: "todo" | "scratchpad" | "process";
  projectId: string;
  id: string;
}

interface SelectionState {
  /** The current text selection, trimmed; `""` when nothing is selected. */
  text: string;
  /** Multi-selected sidebar items, in the order they were picked. */
  items: SelectedItem[];
}

export const useSelectionStore = create<SelectionState>(() => ({
  text: "",
  items: [],
}));

const sameItem = (a: SelectedItem, b: SelectedItem) =>
  a.kind === b.kind && a.id === b.id;

/** Add or remove one item (Cmd/Ctrl+click on a row). */
export function toggleSelectedItem(item: SelectedItem): void {
  const { items } = useSelectionStore.getState();
  const next = items.filter((i) => !sameItem(i, item));
  useSelectionStore.setState({
    items: next.length === items.length ? [...items, item] : next,
  });
}

/** Add items that aren't selected yet (Shift+click range). */
export function addSelectedItems(add: SelectedItem[]): void {
  const { items } = useSelectionStore.getState();
  const fresh = add.filter((a) => !items.some((i) => sameItem(i, a)));
  if (fresh.length > 0) {
    useSelectionStore.setState({ items: [...items, ...fresh] });
  }
}

/** Drop a whole kind's selection for one project (the "clear" buttons). */
export function clearSelectedItems(
  kind: SelectedItem["kind"],
  projectId: string,
): void {
  const { items } = useSelectionStore.getState();
  const next = items.filter(
    (i) => i.kind !== kind || i.projectId !== projectId,
  );
  if (next.length !== items.length) useSelectionStore.setState({ items: next });
}

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
