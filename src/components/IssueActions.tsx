/** Row actions for one issue: open in browser, start an agent, turn into a to-do. */

import { useState } from "react";

import type { IssueInfo, ProjectId } from "../ipc/types";
import {
  issueAgentName,
  issueAgentPrompt,
  issueToTodo,
  useHasIssueTodo,
} from "../lib/issueActions";
import { openExternalLink } from "../lib/links";
import { AgentIcon, CheckIcon, ExternalLinkIcon, TodoIcon } from "./icons";
import { NewAgentModal } from "./NewAgentModal";

export function IssueActions({
  projectId,
  issue,
  className,
}: {
  projectId: ProjectId;
  issue: IssueInfo;
  /** Button style: the sidebar's hover-revealed action, or the pane's own. */
  className: string;
}) {
  const [agentOpen, setAgentOpen] = useState(false);
  const hasTodo = useHasIssueTodo(projectId, issue.url);
  const label = `#${issue.number}`;

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
        title="Start an agent on this issue"
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
          onClick={() => void issueToTodo(projectId, issue)}
        >
          <TodoIcon size={13} />
        </button>
      )}
      {agentOpen && (
        <NewAgentModal
          open
          projectId={projectId}
          initialName={issueAgentName(issue)}
          initialPrompt={issueAgentPrompt(issue)}
          onClose={() => setAgentOpen(false)}
        />
      )}
    </>
  );
}
