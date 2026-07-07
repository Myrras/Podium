/**
 * Read-only list of the to-dos an agent is currently working on, shown under
 * its row in the sidebar. Each entry has a single (x) that unassigns the to-do
 * and sends the agent a best-effort cancel/rollback request. The list is
 * driven by the shared to-do state (assignments arrive/leave via the
 * `todo:changed` refresh), so it updates as agents self-assign over MCP.
 */

import { useMemo } from "react";

import type { ProcessId, ProjectId, TodoInfo } from "../ipc/types";
import { useTodoStore } from "../state/todoStore";
import { CloseIcon } from "./icons";
import styles from "./AgentTodoList.module.css";

/** Stable empty list so the selector doesn't re-render on every store set. */
const NO_TODOS: TodoInfo[] = [];

interface AgentTodoListProps {
  projectId: ProjectId;
  processId: ProcessId;
}

export function AgentTodoList({ projectId, processId }: AgentTodoListProps) {
  // Select the raw project list (stable reference) and derive the filtered
  // subset with useMemo — filtering inside the selector would mint a new array
  // on every store change and trip useSyncExternalStore's snapshot check.
  const todos = useTodoStore((s) => s.todosByProject[projectId] ?? NO_TODOS);
  const unassignTodo = useTodoStore((s) => s.unassignTodo);
  const assigned = useMemo(
    () => todos.filter((t) => t.assignedAgent?.processId === processId),
    [todos, processId],
  );

  if (assigned.length === 0) return null;

  return (
    <ul className={styles.list} aria-label="To-dos this agent is working on">
      {assigned.map((todo) => (
        <li key={todo.id} className={styles.item}>
          <span className={styles.text} title={todo.text}>
            {todo.text}
          </span>
          <button
            type="button"
            className={styles.remove}
            aria-label={`Stop this agent and unassign "${todo.text}"`}
            title="Stop working on this to-do (asks the agent to cancel & roll back)"
            onClick={(e) => {
              e.stopPropagation();
              void unassignTodo(projectId, todo.id);
            }}
          >
            <CloseIcon size={11} />
          </button>
        </li>
      ))}
    </ul>
  );
}
