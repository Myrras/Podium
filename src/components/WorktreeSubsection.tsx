/**
 * "Worktrees" subsection for one sidebar project group, directly under
 * Terminals. Lists Podium-managed git worktrees with their own terminals
 * nested underneath (via the shared `ProcessRow`, so status/click/rename all
 * work as usual) — those terminals are excluded from the plain Terminals
 * list above. No header "+": worktrees come from `create_worktree` over MCP
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

import {
  toIpcError,
  worktreeOpenInWarp,
  worktreeRemove,
} from "../ipc/commands";
import type { ProcessInfo, ProjectId, WorktreeInfo } from "../ipc/types";
import { isActive } from "../lib/processStatus";
import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";
import { useSettingsStore } from "../state/settingsStore";
import { toastError } from "../state/toastStore";
import { useWorktreeStore } from "../state/worktreeStore";
import { ProcessRow } from "./ProcessRow";
import {
  BranchIcon,
  DeleteIcon,
  ExternalLinkIcon,
  RunIcon,
  TerminalIcon,
} from "./icons";
import {
  useSectionCollapsed,
  useWaitingCount,
} from "../lib/useSectionCollapsed";
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
  /** All terminal processes for the project; nested per worktree by `p.worktree`. */
  terminals: ProcessInfo[];
  /** Open the Commands modal targeted at this worktree. */
  onOpenCommands: (worktree: WorktreeInfo) => void;
}

export function WorktreeSubsection({
  projectId,
  projectRoot,
  terminals,
  onOpenCommands,
}: WorktreeSubsectionProps) {
  const worktrees = useWorktreeStore(
    (s) => s.worktreesByProject[projectId] ?? NO_WORKTREES,
  );
  const refresh = useWorktreeStore((s) => s.refresh);
  const setWorktrees = useWorktreeStore((s) => s.setWorktrees);
  const processes = useProcessStore((s) => s.processes);
  const addProcess = useProcessStore((s) => s.addProcess);
  const startProcess = useProcessStore((s) => s.startProcess);

  /** Name of the worktree whose removal is in flight — git can take a
   * moment, so its delete button is disabled until it resolves. */
  const [removing, setRemoving] = useState<string | null>(null);

  // Initial pull; later changes arrive via the `worktree:changed` refresh.
  useEffect(() => {
    void refresh(projectId);
  }, [projectId, refresh]);

  const [collapsed, setCollapsed] = useSectionCollapsed(
    projectRoot,
    "worktrees",
  );
  // Terminals nested under a worktree row hide with it when collapsed.
  const waiting = useWaitingCount(
    terminals
      .filter((p) => p.worktree != null && p.status.state === "running")
      .map((p) => p.id),
  );

  if (worktrees.length === 0) return null;

  const openTerminal = async (wt: WorktreeInfo) => {
    // Same as every other sidebar action: acting on a project activates it.
    useProjectStore.getState().setActiveProject(projectId);
    const shell = useSettingsStore.getState().terminal.shell.trim();
    const info = await addProcess(projectId, {
      name: `Terminal (${wt.name})`,
      kind: "terminal",
      cwd: `.podium/worktrees/${wt.name}`,
      ...(shell ? { command: shell } : {}),
    });
    if (info) await startProcess(info.id);
  };

  const removeWorktree = async (wt: WorktreeInfo) => {
    if (removing) return;
    setRemoving(wt.name);
    try {
      try {
        setWorktrees(
          projectId,
          await worktreeRemove(projectId, wt.name, false),
        );
      } catch (e: unknown) {
        const err = toIpcError(e);
        if (err.kind !== "worktreeDirty") {
          toastError("Could not remove worktree", err.message);
          return;
        }
        const discard = window.confirm(
          `Worktree "${wt.name}" has uncommitted changes. Discard them and remove it anyway?`,
        );
        if (!discard) return;
        setWorktrees(projectId, await worktreeRemove(projectId, wt.name, true));
      }
    } catch (e2: unknown) {
      toastError("Could not remove worktree", toIpcError(e2).message);
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
          waiting={waiting}
          collapsed={collapsed}
          onToggle={() => setCollapsed(!collapsed)}
        />
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
                      onClick={() => void openTerminal(wt)}
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
                      onClick={() => void removeWorktree(wt)}
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
