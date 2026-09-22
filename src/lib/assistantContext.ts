/**
 * What the user currently has selected in Podium, offered to the assistant
 * chat as context — so a prompt can say "these to-dos" instead of pasting ids.
 *
 * Two kinds of selection feed it, both from `lib/selection`:
 * - **Sidebar items** picked with Cmd/Ctrl+click (to-dos, scratchpads,
 *   processes) — every one of them, in pick order. Merely opening an item is
 *   not a selection; see the note there.
 * - **Selected text**.
 *
 * Context rides along as an ordinary `text` attachment: the backend already
 * folds those into the turn ("[Attached: <label>]\n<text>"), so nothing on
 * the Rust side had to learn about selections.
 */

import type { Attachment } from "../ipc/types";
import { useProcessStore } from "../state/processStore";
import { useScratchpadStore } from "../state/scratchpadStore";
import { useTodoStore } from "../state/todoStore";
import { type SelectedItem, useSelectionStore } from "./selection";

/** One piece of context: a chip in the composer, an attachment on send. */
export interface ContextItem {
  /** Stable while the selection is unchanged, so a dismissal sticks. */
  key: string;
  /** Chip text, and the attachment's label in the turn. */
  label: string;
  /** What the assistant is actually handed. */
  text: string;
}

/** Turn a context item into the attachment that carries it. */
export function contextAttachment(item: ContextItem): Attachment {
  return { kind: "text", name: item.label, text: item.text };
}

/** Everything the user has selected right now, in chip order. */
export function useAssistantContext(): ContextItem[] {
  const selectedItems = useSelectionStore((s) => s.items);
  const processes = useProcessStore((s) => s.processes);
  const todosByProject = useTodoStore((s) => s.todosByProject);
  const scratchpadsByProject = useScratchpadStore(
    (s) => s.scratchpadsByProject,
  );
  const selectedText = useSelectionStore((s) => s.text);

  const items: ContextItem[] = [];

  // A selection outlives the row that made it (the project closes, the to-do
  // is deleted), so anything that no longer resolves is simply dropped rather
  // than shown as a chip naming a bare id.
  for (const sel of selectedItems) {
    const item = describe(sel, {
      processes,
      todosByProject,
      scratchpadsByProject,
    });
    if (item) items.push(item);
  }

  if (selectedText) {
    items.push({
      key: `text:${selectedText}`,
      label: "Selected text",
      text: `Text the user has selected in Podium:\n\n${selectedText}`,
    });
  }

  return items;
}

function describe(
  sel: SelectedItem,
  stores: {
    processes: ReturnType<typeof useProcessStore.getState>["processes"];
    todosByProject: ReturnType<typeof useTodoStore.getState>["todosByProject"];
    scratchpadsByProject: ReturnType<
      typeof useScratchpadStore.getState
    >["scratchpadsByProject"];
  },
): ContextItem | null {
  if (sel.kind === "todo") {
    const todo = stores.todosByProject[sel.projectId]?.find(
      (t) => t.id === sel.id,
    );
    if (!todo) return null;
    return {
      key: `todo:${sel.id}`,
      label: `To-do: ${todo.text}`,
      text: `The user has this to-do selected in Podium: "${todo.text}" — to-do id ${sel.id}, project id ${sel.projectId}.`,
    };
  }

  if (sel.kind === "scratchpad") {
    const pad = stores.scratchpadsByProject[sel.projectId]?.find(
      (s) => s.id === sel.id,
    );
    if (!pad) return null;
    return {
      key: `scratchpad:${sel.id}`,
      label: `Scratchpad: ${pad.title}`,
      text: `The user has this scratchpad selected in Podium: "${pad.title}" — scratchpad id ${sel.id}, project id ${sel.projectId}.`,
    };
  }

  const process = stores.processes.find((p) => p.id === sel.id);
  if (!process) return null;
  const kind = process.kind.kind;
  return {
    key: `process:${sel.id}`,
    label: `${kind === "agent" ? "Agent" : "Process"}: ${process.name}`,
    text: `The user has this ${kind} selected in Podium: "${process.name}" — process id ${sel.id}, project id ${process.projectId}.`,
  };
}
