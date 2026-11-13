/** Pinboard popover: the project's to-do list; a to-do opens inside it. */
import type { ProjectId, TodoId } from "../../ipc/types";
import { useFloorStore } from "../../state/floorStore";
import { useProcessStore } from "../../state/processStore";
import { useProjectStore } from "../../state/projectStore";
import { useTodoStore } from "../../state/todoStore";
import { Popover, PopoverSlot } from "../Popover";
import { TodoDetailPane } from "../TodoDetailPane";
import { TodoSubsection } from "../TodoSubsection";
import styles from "./TodoFixturePopover.module.css";

export function TodoFixturePopover({
  projectId,
  itemId,
  anchor,
}: {
  projectId: ProjectId;
  itemId: string | null;
  anchor: HTMLElement | null;
}) {
  const root = useProjectStore(
    (s) => s.projects.find((p) => p.id === projectId)?.root ?? "",
  );
  const todo = useTodoStore((s) =>
    itemId
      ? s.todosByProject[projectId]?.find((t) => t.id === itemId)
      : undefined,
  );
  // Same number as the pinboard's label: open to-dos.
  const openCount = useTodoStore(
    (s) => s.todosByProject[projectId]?.filter((t) => !t.done).length ?? 0,
  );
  const { closePopover, setPopoverItem, setAgentModal, openTerminal } =
    useFloorStore.getState();

  // Same rule as the list's Start: plain click starts the default agent,
  // Cmd/Ctrl-click opens the New agent screen. (No shared helper exists.)
  const agent = todo?.assignedAgent;
  const footerActions = (
    <>
      {agent && agent.active !== false ? (
        <button
          type="button"
          onClick={() => {
            useProcessStore.getState().setActiveProcess(agent.processId);
            openTerminal(agent.processId);
          }}
        >
          Open the agent
        </button>
      ) : (
        <button
          type="button"
          onClick={(e) => {
            if (!todo) return;
            useProjectStore.getState().setActiveProject(projectId);
            if (e.metaKey || e.ctrlKey) {
              setAgentModal({
                projectId,
                todoIds: [todo.id],
                initialName: todo.text,
              });
            } else {
              void useProcessStore
                .getState()
                .spawnAgent(projectId, { todoIds: [todo.id] });
            }
          }}
        >
          Start agent
        </button>
      )}
      <button
        type="button"
        onClick={() => {
          if (!todo) return;
          void useTodoStore
            .getState()
            .setTodoArchived(projectId, todo.id, true);
          setPopoverItem(null);
        }}
      >
        Archive this to-do
      </button>
    </>
  );

  return (
    <Popover
      anchor={anchor}
      title={itemId ? undefined : "To-dos"}
      count={itemId ? undefined : openCount}
      width={itemId ? 520 : 380}
      flush
      onClose={closePopover}
      onBack={itemId ? () => setPopoverItem(null) : undefined}
      backLabel="Back to to-dos"
    >
      {/* Closing unmounts the pane: its autosave flushes on unmount. */}
      {itemId ? (
        <div className={styles.item}>
          <TodoDetailPane
            key={itemId}
            projectId={projectId}
            todoId={itemId as TodoId}
            onClose={() => setPopoverItem(null)}
          />
          {todo && <PopoverSlot name="footer">{footerActions}</PopoverSlot>}
        </div>
      ) : (
        <TodoSubsection
          bare
          projectId={projectId}
          projectRoot={root}
          onOpenTodo={(_, todoId) => setPopoverItem(todoId)}
          onPickAgent={(pid, todoIds, initialName) =>
            setAgentModal({ projectId: pid, todoIds, initialName })
          }
        />
      )}
    </Popover>
  );
}
