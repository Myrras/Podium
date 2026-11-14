/** The ⋯ menu of a room's nameplate: rename, colour, worktrees, board, reload, remove. */
import { useCallback, useEffect, useRef, useState } from "react";

import type { ProjectId, WorktreeInfo } from "../../ipc/types";
import { themeAccentHex } from "../../lib/color";
import { openProcess } from "../../lib/floor/menus";
import { commitProjectName } from "../../lib/projectName";
import { newTerminal } from "../../lib/terminals";
import { dirtyWorktreeQuestion, removeWorktree } from "../../lib/worktrees";
import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";
import { useProcessStore } from "../../state/processStore";
import { useProjectStore } from "../../state/projectStore";
import { useThemeStore } from "../../state/themeStore";
import { useWorktreeStore } from "../../state/worktreeStore";
import { followCamera } from "../../lib/floor/follow";
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
  const { setProjectColor, closeProject, reloadProjectConfig } =
    useProjectStore.getState();
  const worktrees = useWorktreeStore((s) => s.worktreesByProject[projectId]);
  const processes = useProcessStore((s) => s.processes);
  const themeMode = useThemeStore((s) => s.mode);
  const { closePopover, setPopoverItem } = useFloorStore.getState();

  const [draft, setDraft] = useState(project?.name ?? "");
  const [confirmRemove, setConfirmRemove] = useState(false);
  // The worktree awaiting a "discard changes" answer, and how to give it.
  const [dirty, setDirty] = useState<string | null>(null);
  const pendingAnswer = useRef<((discard: boolean) => void) | null>(null);
  // Closing the popover answers "keep", so removeWorktree never hangs.
  useEffect(() => () => pendingAnswer.current?.(false), []);
  const [removing, setRemoving] = useState(false);
  // An edited name not yet saved; Esc closes the popover without a blur, so unmount saves it.
  const pendingName = useRef<string | null>(null);
  const rename = () => {
    const next = pendingName.current;
    pendingName.current = null;
    const current = useProjectStore
      .getState()
      .projects.find((p) => p.id === projectId);
    if (next !== null && current) commitProjectName(current, next);
  };
  const renameRef = useRef(rename);
  renameRef.current = rename;
  useEffect(() => () => renameRef.current(), []);

  // Commit on the native `change` event: React's onChange fires on every drag tick.
  // The input is keyed by `shown`, so it remounts with the current colour (e.g. after No colour).
  // A callback ref gives every input instance exactly one listener, also one
  // that mounts while nothing else changed (e.g. its open worktree vanished).
  const color = project?.color ?? null;
  const shown = color ?? themeAccentHex(themeMode);
  const colorNow = useRef(color);
  colorNow.current = color;
  const colorInput = useCallback(
    (el: HTMLInputElement | null) => {
      if (!el) return;
      const commit = () => {
        if (el.value !== colorNow.current)
          void setProjectColor(projectId, el.value);
      };
      el.addEventListener("change", commit);
      return () => el.removeEventListener("change", commit);
    },
    [projectId, setProjectColor],
  );

  if (!project) return null;
  const close = closePopover;
  const wt = itemId ? worktrees?.find((w) => w.name === itemId) : undefined;
  const agentOf = (w: WorktreeInfo) =>
    processes.find(
      (p) =>
        p.projectId === projectId &&
        p.kind.kind === "agent" &&
        p.worktree === w.name,
    );

  if (wt) {
    const agent = agentOf(wt);
    // Asks inline; the buttons resolve it.
    const confirmForce = (name: string) =>
      new Promise<boolean>((resolve) => {
        setRemoving(false);
        pendingAnswer.current?.(false);
        pendingAnswer.current = resolve;
        setDirty(name);
      });
    const answer = (discard: boolean) => {
      pendingAnswer.current?.(discard);
      pendingAnswer.current = null;
      setDirty(null);
      if (discard) setRemoving(true);
    };
    const remove = async () => {
      if (removing) return;
      setRemoving(true);
      try {
        if (await removeWorktree(projectId, wt.name, confirmForce))
          setPopoverItem(null);
      } finally {
        setRemoving(false);
      }
    };
    return (
      <Popover
        follow={followCamera}
        anchor={anchor}
        title={wt.name}
        onClose={close}
        onBack={() => {
          answer(false);
          setPopoverItem(null);
        }}
        backLabel="Back to the project menu"
      >
        <div className={styles.menu}>
          {agent && (
            <button
              type="button"
              className="ghost"
              onClick={() => openProcess(agent.id)}
            >
              Open the agent
            </button>
          )}
          {/* With terminals off the terminal would get no desk and never show. */}
          {project.sections.terminals && (
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
          )}
          {!wt.inUse &&
            (dirty === wt.name ? (
              <div className={styles.confirm}>
                <span>{dirtyWorktreeQuestion(wt.name)}</span>
                <div className={styles.row}>
                  <button
                    type="button"
                    className={styles.danger}
                    disabled={removing}
                    onClick={() => answer(true)}
                  >
                    Discard and remove
                  </button>
                  <button
                    type="button"
                    className="ghost"
                    onClick={() => answer(false)}
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
                onClick={() => void remove()}
              >
                Remove worktree
              </button>
            ))}
        </div>
      </Popover>
    );
  }

  return (
    <Popover
      follow={followCamera}
      anchor={anchor}
      title={project.name}
      onClose={close}
    >
      <div className={styles.menu}>
        <label className={styles.row}>
          <span>Name</span>
          <input
            type="text"
            aria-label="Project name"
            value={draft}
            onChange={(e) => {
              setDraft(e.target.value);
              pendingName.current = e.target.value;
            }}
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
              key={shown}
              ref={colorInput}
              type="color"
              className={styles.colorInput}
              aria-label="Project colour"
              defaultValue={shown}
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
