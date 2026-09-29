/** Shared helpers for the Git board's card actions: agent prefill and "turn into to-do". */

import { todoAdd, toIpcError } from "../ipc/commands";
import type {
  Approvals,
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

/**
 * The approval badge of an open PR/MR: `2/2`, or `1` when the platform does
 * not say how many are needed; null when there is nothing worth showing.
 */
export function approvalBadge(
  approvals: Approvals | null | undefined,
  me: string | null | undefined,
): { text: string; mine: boolean; title: string } | null {
  if (!approvals) return null;
  const { by, required } = approvals;
  if (by.length === 0 && required === null) return null;
  const mine = !!me && by.includes(me);
  const names = by.map((n) => (n === me ? "you" : n)).join(", ");
  return {
    text: required === null ? `${by.length}` : `${by.length}/${required}`,
    mine,
    title: by.length ? `Approved by ${names}` : "No approvals yet",
  };
}

/** Case-insensitive board search over ref, title, labels and people. */
export function matchesSearch(
  i: BoardItem,
  ref: string,
  query: string,
): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return [
    ref,
    i.title,
    i.author,
    ...i.labels,
    ...(i.assignees ?? []),
    ...i.linked.flatMap((p) => [`#${p.number}`, `!${p.number}`, p.title]),
  ].some((s) => s.toLowerCase().includes(q));
}

/** Ref of a linked PR/MR: `!n` on GitLab, `#n` on GitHub. */
export const prRef = (number: number, provider: IssueProvider) =>
  provider === "gitlab" ? `!${number}` : `#${number}`;

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
