/** Right-click menu contents for floor items. Rename lives in the modal and detail panes. */

import type { MenuConfirm, MenuItem } from "../../components/ContextMenu";
import type {
  ProcessInfo,
  ProjectId,
  ScratchpadInfo,
  TodoInfo,
} from "../../ipc/types";
import { useFloorStore } from "../../state/floorStore";
import { useProcessStore } from "../../state/processStore";
import { useProjectStore } from "../../state/projectStore";
import { useScratchpadStore } from "../../state/scratchpadStore";
import { useTodoStore } from "../../state/todoStore";
import type { ProcessActivity } from "../../state/agentActivityStore";
import { isActive, pillState } from "../processStatus";
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

/** Close agent / Close terminal: stop if active, remove, and hide its modal. */
export function closeProcess(process: ProcessInfo): void {
  const ps = useProcessStore.getState();
  const { id } = process;
  void (async () => {
    if (isActive(process.status)) await ps.stopProcess(id);
    await ps.removeProcess(id);
  })();
  const floor = useFloorStore.getState();
  if (floor.terminalId === id) floor.closeTerminal();
}

/** Agent, terminal or service, by the pill its desk shows: not started, exited and crashed are ended. */
export function processMenu(
  process: ProcessInfo,
  activity: ProcessActivity,
): MenuItem[] {
  const { id } = process;
  const ps = () => useProcessStore.getState();
  const kind = process.kind.kind;
  const active = !ENDED.has(pillState(process.status, kind, activity));
  const items: MenuItem[] = [
    { label: "Open", onSelect: () => openProcess(id) },
    active
      ? {
          label: "Stop",
          confirm: stopConfirm(process),
          onSelect: () => void ps().stopProcess(id),
        }
      : {
          label: kind === "agent" ? "Resume" : "Start",
          onSelect: () => void ps().startProcess(id),
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
      onSelect: () => closeProcess(process),
    });
  return items;
}

export function todoMenu(projectId: ProjectId, todo: TodoInfo): MenuItem[] {
  const agent = todo.assignedAgent;
  const agentItem: MenuItem =
    agent && agent.active !== false
      ? {
          label: "Open the agent",
          onSelect: () => openProcess(agent.processId),
        }
      : {
          label: "Start agent",
          onSelect: () => {
            useProjectStore.getState().setActiveProject(projectId);
            void useProcessStore
              .getState()
              .spawnAgent(projectId, { todoIds: [todo.id] });
          },
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
