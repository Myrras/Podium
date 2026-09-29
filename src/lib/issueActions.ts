/** Shared helpers for the issue row/pane actions: agent prefill and "turn into to-do". */

import { todoAdd, toIpcError } from "../ipc/commands";
import type { IssueInfo, ProjectId } from "../ipc/types";
import { toastError } from "../state/toastStore";
import { useTodoStore } from "../state/todoStore";

export const issueAgentName = (i: IssueInfo) => `#${i.number} ${i.title}`;

export const issueAgentPrompt = (i: IssueInfo) =>
  `Work on issue ${i.url}\n\n# ${i.title}\n\n${i.body}`.trimEnd();

/** True while an active (non-archived) to-do in the project links to this issue. */
export function useHasIssueTodo(projectId: ProjectId, url: string): boolean {
  return useTodoStore((s) =>
    (s.todosByProject[projectId] ?? []).some(
      (t) => !t.archived && t.links.some((l) => l.url === url),
    ),
  );
}

/** todoAdd → todoUpdate(description) → todoAddLink; the store reconciles via `todo:changed`. */
export async function issueToTodo(
  projectId: ProjectId,
  issue: IssueInfo,
): Promise<void> {
  const store = useTodoStore.getState();
  try {
    const todo = await todoAdd(projectId, issueAgentName(issue));
    // Insert eagerly so the follow-up store actions have something to update.
    useTodoStore.setState((s) => ({
      todosByProject: {
        ...s.todosByProject,
        [projectId]: [...(s.todosByProject[projectId] ?? []), todo],
      },
    }));
    if (issue.body.trim()) {
      await store.updateTodo(projectId, todo.id, { description: issue.body });
    }
    await store.addLink(projectId, todo.id, issue.url, `#${issue.number}`);
  } catch (e) {
    toastError("Could not create to-do", toIpcError(e).message);
  }
}
