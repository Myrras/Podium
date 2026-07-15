/**
 * Read-only list of the scratchpads an agent is currently working on, shown
 * under its row in the sidebar. Each entry has a single (x) that unassigns
 * the scratchpad (the backend sends the agent a best-effort cancel/rollback
 * request before clearing the link). The list is driven by the shared
 * scratchpad state (assignments arrive/leave via the `scratchpad:changed`
 * refresh), so it updates as agents self-assign at spawn time.
 *
 * Unlike `AgentTodoList`, clicking a title here navigates to that scratchpad
 * in the work area — scratchpad content is long-lived context worth opening,
 * whereas to-dos stay non-clickable here by design.
 */

import { useMemo } from "react";

import type { ProcessId, ProjectId, ScratchpadInfo } from "../ipc/types";
import { useLayoutStore } from "../state/layoutStore";
import { useProjectStore } from "../state/projectStore";
import { useScratchpadStore } from "../state/scratchpadStore";
import { CloseIcon } from "./icons";
import styles from "./AgentTodoList.module.css";

/** Stable empty list so the selector doesn't re-render on every store set. */
const NO_SCRATCHPADS: ScratchpadInfo[] = [];

interface AgentScratchpadListProps {
  projectId: ProjectId;
  processId: ProcessId;
}

export function AgentScratchpadList({
  projectId,
  processId,
}: AgentScratchpadListProps) {
  // Select the raw project list (stable reference) and derive the filtered
  // subset with useMemo — filtering inside the selector would mint a new array
  // on every store change and trip useSyncExternalStore's snapshot check.
  const scratchpads = useScratchpadStore(
    (s) => s.scratchpadsByProject[projectId] ?? NO_SCRATCHPADS,
  );
  const unassignScratchpad = useScratchpadStore((s) => s.unassignScratchpad);
  const setActiveProject = useProjectStore((s) => s.setActiveProject);
  const openScratchpadInWorkArea = useLayoutStore(
    (s) => s.openScratchpadInWorkArea,
  );
  const assigned = useMemo(
    () => scratchpads.filter((sp) => sp.assignedAgent?.processId === processId),
    [scratchpads, processId],
  );

  if (assigned.length === 0) return null;

  return (
    <ul
      className={styles.list}
      aria-label="Scratchpads this agent is working on"
    >
      {assigned.map((sp) => (
        <li key={sp.id} className={styles.item}>
          <button
            type="button"
            className={styles.text}
            title={`Open "${sp.title}"`}
            onClick={() => {
              setActiveProject(projectId);
              openScratchpadInWorkArea(projectId, sp.id);
            }}
          >
            {sp.title}
          </button>
          <button
            type="button"
            className={styles.remove}
            aria-label={`Stop this agent and unassign "${sp.title}"`}
            title="Stop working on this scratchpad (asks the agent to cancel & roll back)"
            onClick={(e) => {
              e.stopPropagation();
              void unassignScratchpad(projectId, sp.id);
            }}
          >
            <CloseIcon size={11} />
          </button>
        </li>
      ))}
    </ul>
  );
}
