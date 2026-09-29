/**
 * Read-only work-area pane for an opened issue. Reads the live issue from
 * `issueStore`, so it shows a "no longer open" note if a refresh drops it.
 */

import type { IssueInfo, ProjectId } from "../ipc/types";
import { useIssueStore } from "../state/issueStore";
import { useLayoutStore } from "../state/layoutStore";
import { CloseIcon, IssueIcon } from "./icons";
import { IssueActions } from "./IssueActions";
import styles from "./IssueDetailPane.module.css";
import { Markdown } from "./Markdown";

const NO_ISSUES: IssueInfo[] = [];

export function IssueDetailPane({
  projectId,
  number,
}: {
  projectId: ProjectId;
  number: number;
}) {
  const issue = useIssueStore((s) =>
    (s.byProject[projectId]?.issues ?? NO_ISSUES).find(
      (i) => i.number === number,
    ),
  );
  const clearOpenIssue = useLayoutStore((s) => s.clearOpenIssue);

  return (
    <div className={styles.pane}>
      <div className={styles.header}>
        <IssueIcon className={styles.kindIcon} />
        <span className={styles.title}>
          #{number} {issue?.title}
        </span>
        {issue && (
          <IssueActions
            projectId={projectId}
            issue={issue}
            className={styles.action}
          />
        )}
        <button
          type="button"
          aria-label="Close issue"
          title="Close"
          onClick={clearOpenIssue}
        >
          <CloseIcon size={14} />
        </button>
      </div>
      {issue ? (
        <div className={styles.content}>
          {issue.labels.length > 0 && (
            <ul className={styles.labels}>
              {issue.labels.map((l) => (
                <li key={l}>{l}</li>
              ))}
            </ul>
          )}
          {issue.body.trim() ? (
            <Markdown>{issue.body}</Markdown>
          ) : (
            <p className={styles.muted}>No description.</p>
          )}
        </div>
      ) : (
        <p className={styles.muted}>
          This issue is no longer open or assigned to you.
        </p>
      )}
    </div>
  );
}
