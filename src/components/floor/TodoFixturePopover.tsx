/** Pinboard popover: the project's to-do list; a to-do opens inside it. */
import type { ProjectId, TodoId } from "../../ipc/types";
import { useFloorStore } from "../../state/floorStore";
import { useProjectStore } from "../../state/projectStore";
import { Popover } from "../Popover";
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
  const { closePopover, setPopoverItem, setAgentModal } =
    useFloorStore.getState();

  return (
    <Popover
      anchor={anchor}
      title="To-dos"
      width={420}
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
