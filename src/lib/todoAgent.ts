/** The one "start an agent on a to-do" rule: the list's Start, the floor to-do footer and the to-do menu. */
import type { ProjectId, TodoId, TodoInfo } from "../ipc/types";
import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";

export type PickAgent = (
  projectId: ProjectId,
  todoIds: TodoId[],
  initialName: string,
) => void;

/**
 * Plain click spawns the default agent on the to-do; Cmd/Ctrl-click calls
 * `pick` to open the New agent screen. Without a click (a menu), it spawns.
 */
export function startAgentOnTodo(
  projectId: ProjectId,
  todo: Pick<TodoInfo, "id" | "text">,
  click?: { metaKey: boolean; ctrlKey: boolean },
  pick?: PickAgent,
): void {
  useProjectStore.getState().setActiveProject(projectId);
  if (pick && (click?.metaKey || click?.ctrlKey)) {
    pick(projectId, [todo.id], todo.text);
    return;
  }
  void useProcessStore.getState().spawnAgent(projectId, { todoIds: [todo.id] });
}
