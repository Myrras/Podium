/**
 * "Issues" subsection for one sidebar project group: the user's open assigned
 * issues in the repo behind the project's `origin` remote. Clicking a row
 * opens the issue in the browser. A refresh in flight never hides the list
 * (stale rows stay, the header icon spins); each failure kind gets its own
 * inline message — no toasts. The store owns the refresh cadence; this only
 * mounts it. A picker chooses which Settings → Issues setup the project uses
 * ("Auto" matches the remote's host); the gear links to those settings.
 */

import { useEffect } from "react";

import { issuesProjectSetupSet, toIpcError } from "../ipc/commands";
import type { IpcError, IssueInfo, ProjectId } from "../ipc/types";
import { openExternalLink } from "../lib/links";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { useIssueStore, type ProjectIssues } from "../state/issueStore";
import { toastError } from "../state/toastStore";
import { IssueIcon, RestartIcon, SettingsIcon } from "./icons";
import sidebarStyles from "./Sidebar.module.css";
import styles from "./TodoSubsection.module.css";

/** Stable empty state so the selector doesn't re-render on every store set. */
const NONE: ProjectIssues = {
  issues: [],
  error: null,
  loading: false,
  fetchedAt: 0,
};

const NO_LABELS: string[] = [];

/** Inline text per error kind; unknown kinds fall back to the (Podium-owned) message. */
function errorText(error: IpcError): string {
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

/** Errors with nothing to retry: the remote or CLI setup has to change first. */
const SETUP_ERRORS = new Set([
  "issuesNoRemote",
  "issuesGhUnavailable",
  "issuesUnsupportedHost",
  "issuesHostMismatch",
  "issuesGitlabNotConfigured",
  "issuesGitlabAuth",
]);

/** Failures the user fixes in Settings → Issues. */
const SETTINGS_ERRORS = new Set([
  "issuesGhUnavailable",
  "issuesUnsupportedHost",
  "issuesHostMismatch",
  "issuesGitlabNotConfigured",
  "issuesGitlabAuth",
]);

const openIssueSettings = () =>
  useSettingsRequestStore.getState().open("issues");

export function IssueSubsection({
  projectId,
  root,
}: {
  projectId: ProjectId;
  /** Project root: the key of the project's setup pick. */
  root: string;
}) {
  const { issues, error, loading, fetchedAt } = useIssueStore(
    (s) => s.byProject[projectId] ?? NONE,
  );
  const refresh = useIssueStore((s) => s.refresh);
  const startAutoRefresh = useIssueStore((s) => s.startAutoRefresh);
  const overview = useIssueStore((s) => s.overview);
  const loadOverview = useIssueStore((s) => s.loadOverview);

  useEffect(() => startAutoRefresh(projectId), [projectId, startAutoRefresh]);
  useEffect(() => void loadOverview(), [loadOverview]);

  const setups = overview?.setups ?? [];
  const picked = overview?.projects[root] ?? "";
  const pick = (setupId: string) =>
    issuesProjectSetupSet(projectId, setupId || undefined)
      .then((o) => useIssueStore.getState().setOverview(o))
      .catch((e) => toastError(toIpcError(e).message));

  // A setup error means the list (if any) is meaningless; a transient one
  // keeps the stale rows under the message.
  const setupError = error !== null && SETUP_ERRORS.has(error.kind);
  const rows: IssueInfo[] = setupError ? [] : issues;
  const firstLoad = loading && fetchedAt === 0;

  return (
    <div className={sidebarStyles.subsection}>
      <div className={sidebarStyles.sectionHeader}>
        <IssueIcon className={sidebarStyles.panelIcon} />
        <span className={sidebarStyles.panelTitle}>Issues</span>
        <button
          type="button"
          className={sidebarStyles.addBtn}
          aria-label="Issue settings"
          title="Issue settings"
          onClick={openIssueSettings}
        >
          <SettingsIcon size={13} />
        </button>
        <button
          type="button"
          className={sidebarStyles.addBtn}
          aria-label="Refresh issues"
          title="Refresh issues"
          aria-busy={loading}
          onClick={() => void refresh(projectId)}
        >
          <RestartIcon
            size={13}
            className={loading ? sidebarStyles.refreshing : undefined}
          />
        </button>
      </div>
      {setups.length > 0 && (
        <select
          className={sidebarStyles.issueSetupPicker}
          aria-label="Issue setup"
          value={picked}
          onChange={(e) => void pick(e.target.value)}
        >
          <option value="">Auto (match remote)</option>
          {setups.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name}
            </option>
          ))}
        </select>
      )}
      {error && (
        <div className={sidebarStyles.placeholder} role="status">
          <span>{errorText(error)}</span>
          {SETTINGS_ERRORS.has(error.kind) && (
            <button type="button" onClick={openIssueSettings}>
              Check Settings
            </button>
          )}
          {!setupError && (
            <button type="button" onClick={() => void refresh(projectId)}>
              Retry
            </button>
          )}
        </div>
      )}
      {rows.length > 0 ? (
        <div className={styles.rows}>
          {rows.map((issue) => (
            <div key={issue.number} className={styles.row}>
              <div className={styles.rowMain}>
                <button
                  type="button"
                  className={styles.textToggle}
                  title={(issue.labels ?? NO_LABELS).join(", ") || issue.title}
                  onClick={() => openExternalLink(issue.url)}
                >
                  <span className={styles.text}>
                    #{issue.number} {issue.title}
                  </span>
                </button>
              </div>
            </div>
          ))}
        </div>
      ) : (
        !error &&
        (firstLoad ? (
          <div className={sidebarStyles.placeholder}>Loading…</div>
        ) : (
          <div className={sidebarStyles.placeholder}>
            No open issues assigned to you.
          </div>
        ))
      )}
    </div>
  );
}
