/** Right-click menu contents for floor items. Rename lives in the modal and detail panes. */

import type { MenuConfirm, MenuItem } from "../../components/ContextMenu";
import { processStop, toIpcError } from "../../ipc/commands";
import type {
  ProcessId,
  ProcessInfo,
  ProjectId,
  ScratchpadInfo,
  TodoInfo,
} from "../../ipc/types";
import { useFloorStore } from "../../state/floorStore";
import { useProcessStore } from "../../state/processStore";
import { useScratchpadStore } from "../../state/scratchpadStore";
import { toastError } from "../../state/toastStore";
import { useTodoStore } from "../../state/todoStore";
import type { ProcessActivity } from "../../state/agentActivityStore";
import { isActive, pillState } from "../processStatus";
import { type PickAgent, startAgentOnTodo } from "../todoAgent";
import type { PillState } from "../processStatus";

const ENDED = new Set<PillState>(["notStarted", "exited", "crashed"]);

/** Pin a process and open it in the modal, as clicking its desk does. */
export function openProcess(id: string): void {
  useProcessStore.getState().setActiveProcess(id);
  useFloorStore.getState().openTerminal(id);
}

/** Confirm texts for Stop and Close, shared by these menus and the terminal modal. */
export function stopConfirm(process: ProcessInfo): MenuConfirm {
  return {
    title: `Stop ${process.name}?`,
    body:
      process.kind.kind === "agent"
        ? "The session ends. You can resume it later."
        : "The process stops. You can start it again later.",
    action: "Stop",
  };
}

export function closeConfirm(process: ProcessInfo): MenuConfirm {
  return {
    title: `Close ${process.name}?`,
    body:
      process.kind.kind === "agent"
        ? "The agent stops and its desk is removed from the floor."
        : "The terminal stops and is removed, with its output.",
    action: "Close",
  };
}

/** The process as the store has it now: a menu or confirm may be older. */
function current(id: ProcessId): ProcessInfo | undefined {
  return useProcessStore.getState().processes.find((p) => p.id === id);
}

/**
 * Close agent / Close terminal: stop if active now, remove, and hide its
 * modal. A failed stop keeps it. Calls the IPC directly: the store's stop
 * swallows the error, and a successful stop still reads as active
 * (`stopping`) until the process exits.
 */
export function closeProcess(id: ProcessId): void {
  void (async () => {
    const process = current(id);
    if (!process) return;
    if (isActive(process.status)) {
      try {
        await processStop(id);
      } catch (e) {
        const err = toIpcError(e);
        // Ended (or already stopping) in between: nothing left to stop.
        if (err.kind !== "processNotRunning") {
          toastError("Could not stop process", err.message);
          return;
        }
      }
    }
    await useProcessStore.getState().removeProcess(id);
  })();
  const floor = useFloorStore.getState();
  if (floor.terminalId === id) floor.closeTerminal();
}

/**
 * Agent, terminal or service, by the pill its desk shows: not started, exited
 * and crashed are ended. Built at right-click time, so each action re-checks
 * the status when it runs and does nothing when the menu is out of date.
 */
export function processMenu(
  process: ProcessInfo,
  activity: ProcessActivity,
): MenuItem[] {
  const { id } = process;
  const ps = () => useProcessStore.getState();
  const kind = process.kind.kind;
  const active = !ENDED.has(pillState(process.status, kind, activity));
  const status = () => current(id)?.status;
  const items: MenuItem[] = [
    { label: "Open", onSelect: () => openProcess(id) },
    active
      ? {
          label: "Stop",
          confirm: stopConfirm(process),
          // Only a running one: stopping one that is already stopping errors.
          onSelect: () => {
            if (status()?.state === "running") void ps().stopProcess(id);
          },
        }
      : {
          label: kind === "agent" ? "Resume" : "Start",
          onSelect: () => {
            const s = status();
            if (s && !isActive(s)) void ps().startProcess(id);
          },
        },
  ];
  if (active && kind !== "agent")
    items.push({
      label: "Restart",
      onSelect: () => void ps().restartProcess(id),
    });
  // podium.yml owns services: no Close.
  if (kind !== "service")
    items.push("separator", {
      label: kind === "agent" ? "Close agent" : "Close terminal",
      danger: true,
      confirm: closeConfirm(process),
      onSelect: () => closeProcess(id),
    });
  return items;
}

/** A to-do's agent action, for its menu and the to-do popover footer: open its live agent, or start one. */
export function todoAgentAction(
  projectId: ProjectId,
  todo: TodoInfo,
): {
  label: "Open the agent" | "Start agent";
  run: (
    click?: { metaKey: boolean; ctrlKey: boolean },
    pick?: PickAgent,
  ) => void;
} {
  const agent = todo.assignedAgent;
  return agent && agent.active !== false
    ? { label: "Open the agent", run: () => openProcess(agent.processId) }
    : {
        label: "Start agent",
        run: (click, pick) => startAgentOnTodo(projectId, todo, click, pick),
      };
}

export function todoMenu(projectId: ProjectId, todo: TodoInfo): MenuItem[] {
  const agent = todoAgentAction(projectId, todo);
  const agentItem: MenuItem = {
    label: agent.label,
    onSelect: () => agent.run(),
  };
  return [
    {
      label: "Open",
      onSelect: () =>
        useFloorStore
          .getState()
          .openPopover({ projectId, kind: "todos", itemId: todo.id }),
    },
    agentItem,
    {
      label: todo.done ? "Mark not done" : "Mark done",
      onSelect: () =>
        void useTodoStore
          .getState()
          .setTodoDone(projectId, todo.id, !todo.done),
    },
    "separator",
    {
      label: "Archive",
      danger: true,
      onSelect: () =>
        void useTodoStore.getState().setTodoArchived(projectId, todo.id, true),
    },
  ];
}

export function scratchpadMenu(
  projectId: ProjectId,
  scratchpad: ScratchpadInfo,
): MenuItem[] {
  const { id, title } = scratchpad;
  return [
    {
      label: "Open",
      onSelect: () =>
        useFloorStore
          .getState()
          .openPopover({ projectId, kind: "scratchpads", itemId: id }),
    },
    {
      label: "Start agent on this",
      onSelect: () =>
        useFloorStore.getState().setScratchpadAgentModal({
          projectId,
          scratchpadIds: [id],
          initialName: title,
        }),
    },
    "separator",
    {
      label: "Archive",
      danger: true,
      onSelect: () =>
        void useScratchpadStore
          .getState()
          .setScratchpadArchived(projectId, id, true),
    },
  ];
}
