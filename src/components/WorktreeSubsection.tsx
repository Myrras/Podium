/**
 * "Worktrees" subsection for one sidebar project group, directly under
 * Terminals. Lists Podium-managed git worktrees with their own terminals
 * nested underneath (via the shared `ProcessRow`, so status/click/rename all
 * work as usual) — those terminals are excluded from the plain Terminals
 * list above. The header's only action is a refresh (re-reads git, for
 * changes made outside Podium); no "+": worktrees come from `create_worktree` over MCP
 * (an agent's own doing), never from the sidebar. Hidden entirely when the
 * project has none, same as the Processes subsection when there are no
 * `podium.yml` services.
 *
 * Per-row actions: open a terminal rooted in the worktree, open the Commands
 * modal targeted at it (owned by the caller — `ProjectGroup` already hosts
 * one for the project root), or remove it (the same dirty-confirm flow the
 * old Worktrees modal used). "In use" is derived live from the process store
 * (any non-exited process with `worktree === wt.name`) rather than the
 * worktree's own cached `inUse` flag, which can go stale between backend
 * refreshes while the sidebar stays live; the backend's `WorktreeInUse`
 * guard remains the authority and surfaces as a toast if it fires anyway.
 */

import { useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";

import { toIpcError, worktreeOpenInWarp } from "../ipc/commands";
import type { ProcessInfo, ProjectId, WorktreeInfo } from "../ipc/types";
import { activityRollup } from "../lib/activityRollup";
import { isActive } from "../lib/processStatus";
import { newTerminal } from "../lib/terminals";
import { dirtyWorktreeQuestion, removeWorktree } from "../lib/worktrees";
import { useAgentActivityStore } from "../state/agentActivityStore";
import { useProcessStore } from "../state/processStore";
import { toastError } from "../state/toastStore";
import { useWorktreeStore } from "../state/worktreeStore";
import { ProcessRow } from "./ProcessRow";
import {
  BranchIcon,
  DeleteIcon,
  ExternalLinkIcon,
  RestartIcon,
  RunIcon,
  TerminalIcon,
} from "./icons";
import { useSectionCollapsed } from "../lib/useSectionCollapsed";
import { SectionTitle } from "./SectionTitle";
import sidebarStyles from "./Sidebar.module.css";
import styles from "./TodoSubsection.module.css";
import ownStyles from "./WorktreeSubsection.module.css";

/** Stable empty list so the selector doesn't re-render on every store set. */
const NO_WORKTREES: WorktreeInfo[] = [];

interface WorktreeSubsectionProps {
  projectId: ProjectId;
  /** Project root: the key its collapse state is persisted under. */
  projectRoot: string;
  /** A Windows-drive project running natively in WSL mode: its terminals
   * skip the (distro) shell setting — see `terminalShellFor`. */
  nativeInWslMode: boolean;
  /** All terminal processes for the project; nested per worktree by `p.worktree`. */
  terminals: ProcessInfo[];
  /** Open the Commands modal targeted at this worktree. */
  onOpenCommands: (worktree: WorktreeInfo) => void;
}

export function WorktreeSubsection({
  projectId,
  projectRoot,
  nativeInWslMode,
  terminals,
  onOpenCommands,
}: WorktreeSubsectionProps) {
  const worktrees = useWorktreeStore(
    (s) => s.worktreesByProject[projectId] ?? NO_WORKTREES,
  );
  const refresh = useWorktreeStore((s) => s.refresh);
  const processes = useProcessStore((s) => s.processes);

  /** Name of the worktree whose removal is in flight — git can take a
   * moment, so its delete button is disabled until it resolves. */
  const [removing, setRemoving] = useState<string | null>(null);
  /** A manual refresh (header button) is in flight. */
  const [refreshing, setRefreshing] = useState(false);

  // Initial pull; later changes arrive via the `worktree:changed` refresh.
  useEffect(() => {
    void refresh(projectId);
  }, [projectId, refresh]);

  const [collapsed, setCollapsed] = useSectionCollapsed(
    projectRoot,
    "worktrees",
  );
  // Terminals nested under a worktree row hide with it when collapsed.
  const { working, needs } = useAgentActivityStore(
    useShallow((s) =>
      activityRollup(
        terminals.filter((p) => p.worktree != null),
        s.details,
      ),
    ),
  );

  if (worktrees.length === 0) return null;

  // Re-reads `git worktree list`, for changes made outside Podium (the
  // `worktree:changed` event only covers create/remove through it).
  const refreshNow = async () => {
    if (refreshing) return;
    setRefreshing(true);
    try {
      await refresh(projectId);
    } finally {
      setRefreshing(false);
    }
  };

  const remove = async (wt: WorktreeInfo) => {
    if (removing) return;
    setRemoving(wt.name);
    try {
      await removeWorktree(projectId, wt.name, (name) =>
        Promise.resolve(window.confirm(dirtyWorktreeQuestion(name))),
      );
    } finally {
      setRemoving(null);
    }
  };

  return (
    <div
      className={sidebarStyles.subsection}
      data-collapsed={collapsed ? "true" : undefined}
    >
      <div className={sidebarStyles.sectionHeader}>
        <SectionTitle
          title="Worktrees"
          Icon={BranchIcon}
          count={worktrees.length}
          working={working}
          needs={needs}
          collapsed={collapsed}
          onToggle={() => setCollapsed(!collapsed)}
        />
        <button
          type="button"
          className={`${sidebarStyles.addBtn} ${sidebarStyles.hoverAction} ${ownStyles.refreshBtn}`}
          aria-label="Refresh worktrees"
          title={refreshing ? "Refreshing…" : "Refresh worktrees"}
          data-busy={refreshing || undefined}
          disabled={refreshing}
          onClick={() => void refreshNow()}
        >
          <RestartIcon size={13} />
        </button>
      </div>
      {!collapsed && (
        <div className={styles.rows}>
          {worktrees.map((wt) => {
            const busy = processes.some(
              (p) => p.worktree === wt.name && isActive(p.status),
            );
            const nested = terminals.filter((p) => p.worktree === wt.name);
            return (
              <div key={wt.name}>
                <div className={styles.row}>
                  <div className={styles.rowMain}>
                    <BranchIcon size={13} className={ownStyles.branchIcon} />
                    <span className={styles.text} title={wt.branch}>
                      {wt.name}
                    </span>
                    <button
                      type="button"
                      className={styles.action}
                      aria-label={`Open terminal in ${wt.name}`}
                      title="Open terminal here"
                      onClick={() =>
                        void newTerminal({ id: projectId, nativeInWslMode }, wt)
                      }
                    >
                      <TerminalIcon size={13} />
                    </button>
                    <button
                      type="button"
                      className={styles.action}
                      aria-label={`Open ${wt.name} in Warp`}
                      title="Open in Warp"
                      onClick={() =>
                        worktreeOpenInWarp(projectId, wt.name).catch(
                          (e: unknown) =>
                            toastError(
                              "Could not open Warp",
                              toIpcError(e).message,
                            ),
                        )
                      }
                    >
                      <ExternalLinkIcon size={13} />
                    </button>
                    <button
                      type="button"
                      className={styles.action}
                      aria-label={`Commands for ${wt.name}`}
                      title="Commands"
                      onClick={() => onOpenCommands(wt)}
                    >
                      <RunIcon size={13} />
                    </button>
                    <button
                      type="button"
                      className={`${styles.remove} ${ownStyles.removeBtn}`}
                      aria-label={`Delete worktree ${wt.name}`}
                      title={
                        busy
                          ? "A process is still running in this worktree"
                          : removing === wt.name
                            ? "Removing…"
                            : "Delete worktree"
                      }
                      data-busy={removing === wt.name || undefined}
                      disabled={busy || removing !== null}
                      onClick={() => void remove(wt)}
                    >
                      <DeleteIcon size={13} />
                    </button>
                  </div>
                </div>
                {nested.length > 0 && (
                  <div className={ownStyles.nested}>
                    {nested.map((p) => (
                      <ProcessRow key={p.id} process={p} />
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
