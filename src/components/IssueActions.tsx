/**
 * Card actions for one board item: open in browser, start an agent, turn into
 * a to-do, and unassign yourself when you are an assignee.
 */

import { useState } from "react";

import type { BoardItem, ProjectId } from "../ipc/types";
import {
  issueAgentName,
  issueAgentPrompt,
  issueToTodo,
  useHasIssueTodo,
} from "../lib/issueActions";
import { openExternalLink } from "../lib/links";
import { useIssueStore } from "../state/issueStore";
import {
  AgentIcon,
  CheckIcon,
  ExternalLinkIcon,
  TodoIcon,
  UnassignIcon,
} from "./icons";
import { NewAgentModal } from "./NewAgentModal";

export function IssueActions({
  projectId,
  issue,
  itemRef: label,
  className,
}: {
  projectId: ProjectId;
  issue: BoardItem;
  /** `#12` / `!41`, see `itemRef`. */
  itemRef: string;
  /** Button style: the card's hover-revealed action, or the drawer's own. */
  className: string;
}) {
  const [agentOpen, setAgentOpen] = useState(false);
  const hasTodo = useHasIssueTodo(projectId, issue.url);
  const me = useIssueStore((s) => s.byProject[projectId]?.board?.me);
  const unassign = useIssueStore((s) => s.unassign);
  const [unassigning, setUnassigning] = useState(false);
  const assignedToMe = !!me && (issue.assignees ?? []).includes(me);

  return (
    <>
      <button
        type="button"
        className={className}
        aria-label={`Open ${label} in browser`}
        title="Open in browser"
        onClick={() => openExternalLink(issue.url)}
      >
        <ExternalLinkIcon size={13} />
      </button>
      <button
        type="button"
        className={className}
        aria-label={`Start an agent on ${label}`}
        title="Start an agent on this"
        onClick={() => setAgentOpen(true)}
      >
        <AgentIcon size={13} />
      </button>
      {hasTodo ? (
        <span
          className={className}
          role="img"
          aria-label={`To-do exists for ${label}`}
          title="To-do exists"
        >
          <CheckIcon size={13} />
        </span>
      ) : (
        <button
          type="button"
          className={className}
          aria-label={`Create a to-do for ${label}`}
          title="Turn into a to-do"
          onClick={() => void issueToTodo(projectId, issue, label)}
        >
          <TodoIcon size={13} />
        </button>
      )}
      {assignedToMe && (
        <button
          type="button"
          className={className}
          aria-label={`Unassign me from ${label}`}
          title="Unassign me"
          disabled={unassigning}
          onClick={() => {
            setUnassigning(true);
            void unassign(projectId, issue).finally(() =>
              setUnassigning(false),
            );
          }}
        >
          <UnassignIcon size={13} />
        </button>
      )}
      {agentOpen && (
        <NewAgentModal
          open
          projectId={projectId}
          initialName={issueAgentName(issue, label)}
          initialPrompt={issueAgentPrompt(issue)}
          onClose={() => setAgentOpen(false)}
        />
      )}
    </>
  );
}
