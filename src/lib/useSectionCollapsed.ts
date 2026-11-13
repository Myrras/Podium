/** Persisted collapse state of one sidebar subsection of a project. */

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
