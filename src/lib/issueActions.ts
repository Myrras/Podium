/** Shared helpers for the Git board's card actions: agent prefill and "turn into to-do". */

import { todoAdd, toIpcError } from "../ipc/commands";
import type {
  BoardItem,
  IpcError,
  IssueProvider,
  ProjectId,
} from "../ipc/types";
import { toastError } from "../state/toastStore";
import { useTodoStore } from "../state/todoStore";

/** `#12` for issues and GitHub PRs, `!41` for GitLab MRs. */
export const itemRef = (i: BoardItem, provider: IssueProvider) =>
  i.kind === "pr" && provider === "gitlab" ? `!${i.number}` : `#${i.number}`;

export const issueAgentName = (i: BoardItem, ref: string) =>
  `${ref} ${i.title}`;

export const issueAgentPrompt = (i: BoardItem) =>
  `Work on ${i.kind === "pr" ? "pull request" : "issue"} ${i.url}\n\n# ${i.title}\n\n${i.body}`.trimEnd();

/** Inline text per board error kind; unknown kinds fall back to the (Podium-owned) message. */
export function boardErrorText(error: IpcError): string {
  switch (error.kind) {
    case "issuesNoRemote":
      return "No GitHub or GitLab remote found.";
    case "issuesGhUnavailable":
      return "GitHub CLI missing or not logged in. Run `gh auth login`.";
    case "issuesUnsupportedHost":
      return "No setup in Settings matches this remote's host.";
    case "issuesGitlabNotConfigured":
      return "This GitLab setup has no token.";
    case "issuesGitlabAuth":
      return "GitLab rejected the token.";
    default:
      return error.message;
  }
}

/** True while an active (non-archived) to-do in the project links to this item. */
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
  item: BoardItem,
  ref: string,
): Promise<void> {
  const store = useTodoStore.getState();
  try {
    const todo = await todoAdd(projectId, issueAgentName(item, ref));
    // Insert eagerly so the follow-up store actions have something to update.
    useTodoStore.setState((s) => ({
      todosByProject: {
        ...s.todosByProject,
        [projectId]: [...(s.todosByProject[projectId] ?? []), todo],
      },
    }));
    if (item.body.trim()) {
      await store.updateTodo(projectId, todo.id, { description: item.body });
    }
    await store.addLink(projectId, todo.id, item.url, ref);
  } catch (e) {
    toastError("Could not create to-do", toIpcError(e).message);
  }
}
