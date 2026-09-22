/**
 * What the user currently has selected in Podium, offered to the assistant
 * chat as context — so a prompt can say "this to-do" instead of pasting an id.
 *
 * Two kinds of selection feed it:
 * - The **item filling the work area** (a to-do, a scratchpad, a process),
 *   already tracked by the layout/process stores — no new selection state.
 * - **Selected text**, tracked by `lib/selection`.
 *
 * Context rides along as an ordinary `text` attachment: the backend already
 * folds those into the turn ("[Attached: <label>]\n<text>"), so nothing on
 * the Rust side had to learn about selections.
 */

import type { Attachment } from "../ipc/types";
import { useLayoutStore } from "../state/layoutStore";
import { useProcessStore } from "../state/processStore";
import { useScratchpadStore } from "../state/scratchpadStore";
import { useTodoStore } from "../state/todoStore";
import { useSelectionStore } from "./selection";

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
  const openTodo = useLayoutStore((s) => s.openTodo);
  const openScratchpad = useLayoutStore((s) => s.openScratchpad);
  const activeProcessId = useProcessStore((s) => s.activeProcessId);
  const processes = useProcessStore((s) => s.processes);
  const todosByProject = useTodoStore((s) => s.todosByProject);
  const scratchpadsByProject = useScratchpadStore(
    (s) => s.scratchpadsByProject,
  );
  const selectedText = useSelectionStore((s) => s.text);

  const items: ContextItem[] = [];

  if (openTodo) {
    const todo = todosByProject[openTodo.projectId]?.find(
      (t) => t.id === openTodo.todoId,
    );
    items.push({
      key: `todo:${openTodo.todoId}`,
      label: `To-do: ${todo?.text ?? openTodo.todoId}`,
      text: `The user has this to-do selected in Podium: "${todo?.text ?? "(not loaded)"}" — to-do id ${openTodo.todoId}, project id ${openTodo.projectId}.`,
    });
  }

  if (openScratchpad) {
    const pad = scratchpadsByProject[openScratchpad.projectId]?.find(
      (s) => s.id === openScratchpad.scratchpadId,
    );
    items.push({
      key: `scratchpad:${openScratchpad.scratchpadId}`,
      label: `Scratchpad: ${pad?.title ?? openScratchpad.scratchpadId}`,
      text: `The user has this scratchpad selected in Podium: "${pad?.title ?? "(not loaded)"}" — scratchpad id ${openScratchpad.scratchpadId}, project id ${openScratchpad.projectId}.`,
    });
  }

  const process = activeProcessId
    ? processes.find((p) => p.id === activeProcessId)
    : undefined;
  if (process) {
    const kind = process.kind.kind === "agent" ? "agent" : process.kind.kind;
    items.push({
      key: `process:${process.id}`,
      label: `${kind === "agent" ? "Agent" : "Process"}: ${process.name}`,
      text: `The user has this ${kind} selected in Podium: "${process.name}" — process id ${process.id}, project id ${process.projectId}.`,
    });
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
