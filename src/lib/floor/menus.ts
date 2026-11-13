/** Right-click menu contents for floor items. Rename lives in the modal and detail panes. */

import type { MenuItem } from "../../components/ContextMenu";
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

function openProcess(id: string): void {
  useProcessStore.getState().setActiveProcess(id);
  useFloorStore.getState().openTerminal(id);
}

/** Agent, terminal or service. */
export function processMenu(
  process: ProcessInfo,
  activity: ProcessActivity,
): MenuItem[] {
  const { id } = process;
  const ps = () => useProcessStore.getState();
  const kind = process.kind.kind;
  const state = pillState(process.status, kind, activity);
  const ended = state === "exited" || state === "crashed";
  const running = process.status.state === "running";
  const open: MenuItem = { label: "Open", onSelect: () => openProcess(id) };

  if (kind === "agent") {
    return [
      open,
      ...(isActive(process.status)
        ? [{ label: "Stop", onSelect: () => void ps().stopProcess(id) }]
        : []),
      ...(ended
        ? [
            {
              label: "Resume session",
              onSelect: () => void ps().startProcess(id),
            },
            "separator" as const,
            {
              label: "Clear the desk",
              danger: true,
              onSelect: () => void ps().removeProcess(id),
            },
          ]
        : []),
    ];
  }

  const items: MenuItem[] = [open];
  if (running) {
    items.push(
      { label: "Stop", onSelect: () => void ps().stopProcess(id) },
      { label: "Restart", onSelect: () => void ps().restartProcess(id) },
    );
  } else if (!isActive(process.status)) {
    items.push({ label: "Start", onSelect: () => void ps().startProcess(id) });
  }
  // podium.yml owns services: no remove.
  if (kind === "terminal") {
    items.push("separator", {
      label: "Close terminal",
      danger: true,
      onSelect: () => {
        void (async () => {
          if (running) await ps().stopProcess(id);
          await ps().removeProcess(id);
        })();
        const floor = useFloorStore.getState();
        if (floor.terminalId === id) floor.closeTerminal();
      },
    });
  }
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
