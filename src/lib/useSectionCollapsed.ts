/** Persisted collapse state of one sidebar subsection of a project. */

import type { ProjectId } from "../ipc/types";
import { type Section, sectionKey, useLayoutStore } from "../state/layoutStore";
import { useProjectStore } from "../state/projectStore";

/** `[collapsed, setCollapsed]` for one subsection of a project. */
export function useSectionCollapsed(
  projectId: ProjectId,
  section: Section,
): [boolean, (collapsed: boolean) => void] {
  const root = useProjectStore(
    (s) => s.projects.find((p) => p.id === projectId)?.root ?? "",
  );
  const collapsed = useLayoutStore(
    (s) => s.collapsedSections[sectionKey(root, section)] ?? false,
  );
  const setSectionCollapsed = useLayoutStore((s) => s.setSectionCollapsed);
  return [collapsed, (next) => setSectionCollapsed(root, section, next)];
}
