/** The ⋯ menu of a room's nameplate: rename, colour, worktrees, terminal, board, reload, remove. */
import { useEffect, useRef, useState } from "react";

import { toIpcError, worktreeRemove } from "../../ipc/commands";
import type { ProjectId, WorktreeInfo } from "../../ipc/types";
import { themeAccentHex } from "../../lib/color";
import { newTerminal } from "../../lib/terminals";
import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";
import { useProcessStore } from "../../state/processStore";
import { useProjectStore } from "../../state/projectStore";
import { useThemeStore } from "../../state/themeStore";
import { toastError } from "../../state/toastStore";
import { useWorktreeStore } from "../../state/worktreeStore";
import { Popover } from "../Popover";
import styles from "./ProjectMenuPopover.module.css";

export function ProjectMenuPopover({
  projectId,
  itemId,
  anchor,
}: {
  projectId: ProjectId;
  itemId: string | null;
  anchor: HTMLElement | null;
}) {
  const project = useProjectStore((s) =>
    s.projects.find((p) => p.id === projectId),
  );
  const { renameProject, setProjectColor, closeProject, reloadProjectConfig } =
    useProjectStore.getState();
  const worktrees = useWorktreeStore((s) => s.worktreesByProject[projectId]);
  const processes = useProcessStore((s) => s.processes);
  const themeMode = useThemeStore((s) => s.mode);
  const { closePopover, setPopoverItem, openTerminal } =
    useFloorStore.getState();

  const [draft, setDraft] = useState(project?.name ?? "");
  const [confirmRemove, setConfirmRemove] = useState(false);
  // Name of the worktree awaiting a "discard changes" answer.
  const [dirty, setDirty] = useState<string | null>(null);
  const [removing, setRemoving] = useState(false);
  const colorRef = useRef<HTMLInputElement>(null);

  // Commit on the native `change` event: React's onChange fires on every drag tick.
  const color = project?.color ?? null;
  useEffect(() => {
    const el = colorRef.current;
    if (!el) return;
    const commit = () => {
      if (el.value !== color) void setProjectColor(projectId, el.value);
    };
    el.addEventListener("change", commit);
    return () => el.removeEventListener("change", commit);
  }, [projectId, color, setProjectColor, itemId]);

  if (!project) return null;
  const close = closePopover;
  const wt = itemId ? worktrees?.find((w) => w.name === itemId) : undefined;

  if (wt) {
    const agent = processes.find(
      (p) =>
        p.projectId === projectId &&
        p.kind.kind === "agent" &&
        p.worktree === wt.name,
    );
    const remove = async (force: boolean) => {
      if (removing) return;
      setRemoving(true);
      try {
        useWorktreeStore
          .getState()
          .setWorktrees(
            projectId,
            await worktreeRemove(projectId, wt.name, force),
          );
        setDirty(null);
        setPopoverItem(null);
      } catch (e: unknown) {
        const err = toIpcError(e);
        if (err.kind === "worktreeDirty" && !force) setDirty(wt.name);
        else toastError("Could not remove worktree", err.message);
      } finally {
        setRemoving(false);
      }
    };
    return (
      <Popover
        anchor={anchor}
        title={wt.name}
        onClose={close}
        onBack={() => {
          setDirty(null);
          setPopoverItem(null);
        }}
        backLabel="Back to the project menu"
      >
        <div className={styles.menu}>
          {agent && (
            <button
              type="button"
              className="ghost"
              onClick={() => openTerminal(agent.id)}
            >
              Open the agent
            </button>
          )}
          <button
            type="button"
            className="ghost"
            onClick={() => {
              void newTerminal(project, wt);
              close();
            }}
          >
            Open a terminal here
          </button>
          {!wt.inUse &&
            (dirty === wt.name ? (
              <div className={styles.confirm}>
                <span>
                  {`Worktree "${wt.name}" has uncommitted changes. Discard them and remove it anyway?`}
                </span>
                <div className={styles.row}>
                  <button
                    type="button"
                    className={styles.danger}
                    disabled={removing}
                    onClick={() => void remove(true)}
                  >
                    Discard and remove
                  </button>
                  <button
                    type="button"
                    className="ghost"
                    onClick={() => setDirty(null)}
                  >
                    Keep
                  </button>
                </div>
              </div>
            ) : (
              <button
                type="button"
                className={`ghost ${styles.danger}`}
                disabled={removing}
                onClick={() => void remove(false)}
              >
                Remove worktree
              </button>
            ))}
        </div>
      </Popover>
    );
  }

  const rename = () => {
    const next = draft.trim();
    if (next === project.name) return;
    void renameProject(projectId, next.length > 0 ? next : null);
  };
  const agentOf = (w: WorktreeInfo) =>
    processes.find(
      (p) =>
        p.projectId === projectId &&
        p.kind.kind === "agent" &&
        p.worktree === w.name,
    );

  return (
    <Popover anchor={anchor} title={project.name} onClose={close}>
      <div className={styles.menu}>
        <label className={styles.row}>
          <span>Name</span>
          <input
            type="text"
            aria-label="Project name"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={rename}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
            }}
          />
        </label>
        <div className={styles.row}>
          <label className={styles.row}>
            <span>Colour</span>
            <input
              ref={colorRef}
              type="color"
              className={styles.colorInput}
              aria-label="Project colour"
              defaultValue={project.color ?? themeAccentHex(themeMode)}
            />
          </label>
          {project.color && (
            <button
              type="button"
              className="ghost"
              onClick={() => void setProjectColor(projectId, null)}
            >
              No colour
            </button>
          )}
        </div>
        {project.sections.worktrees && worktrees && worktrees.length > 0 && (
          <>
            <h3>Worktrees</h3>
            {worktrees.map((w) => (
              <button
                key={w.name}
                type="button"
                className="ghost"
                onClick={() => setPopoverItem(w.name)}
              >
                {w.name}
                <span className={styles.sub}>{agentOf(w)?.name ?? "Free"}</span>
              </button>
            ))}
          </>
        )}
        {project.sections.terminals && (
          <button
            type="button"
            className="ghost"
            onClick={() => {
              void newTerminal(project);
              close();
            }}
          >
            New terminal
          </button>
        )}
        {project.sections.issues && (
          <button
            type="button"
            className="ghost"
            onClick={() => {
              useLayoutStore.getState().openBoardInWorkArea(projectId);
              close();
            }}
          >
            Git board
          </button>
        )}
        <button
          type="button"
          className="ghost"
          onClick={() => void reloadProjectConfig(projectId)}
        >
          Reload podium.yml
        </button>
        {confirmRemove ? (
          <div className={styles.confirm}>
            <span>{`Remove ${project.name} from Podium? The folder stays on disk.`}</span>
            <div className={styles.row}>
              <button
                type="button"
                className={styles.danger}
                onClick={() => {
                  close();
                  void closeProject(projectId);
                }}
              >
                Remove
              </button>
              <button
                type="button"
                className="ghost"
                onClick={() => setConfirmRemove(false)}
              >
                Keep
              </button>
            </div>
          </div>
        ) : (
          <button
            type="button"
            className={`ghost ${styles.danger}`}
            onClick={() => setConfirmRemove(true)}
          >
            Remove project
          </button>
        )}
      </div>
    </Popover>
  );
}
