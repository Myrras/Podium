/** Persisted collapse state of one sidebar subsection of a project. */

import type { ProcessId } from "../ipc/types";
import { useAgentActivityStore } from "../state/agentActivityStore";
import { type Section, sectionKey, useLayoutStore } from "../state/layoutStore";

/**
 * `[collapsed, setCollapsed]` for one subsection of the project at `root`.
 * Keyed by root (not project id) so the state survives a restart.
 */
export function useSectionCollapsed(
  root: string,
  section: Section,
): [boolean, (collapsed: boolean) => void] {
  const collapsed = useLayoutStore(
    (s) => s.collapsedSections[sectionKey(root, section)] ?? false,
  );
  const setSectionCollapsed = useLayoutStore((s) => s.setSectionCollapsed);
  return [collapsed, (next) => setSectionCollapsed(root, section, next)];
}

/**
 * How many of `ids` are waiting on the user ("needs input"), so a collapsed
 * subsection can still say that one of its hidden rows is stuck.
 */
export function useWaitingCount(ids: readonly ProcessId[]): number {
  return useAgentActivityStore(
    (s) => ids.filter((id) => s.activity[id] === "waiting").length,
  );
}
