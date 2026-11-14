/** The project rename rule shared by the sidebar and the Floor project menu. */

import type { ProjectInfo } from "../ipc/types";
import { useProjectStore } from "../state/projectStore";

/**
 * Save `draft` as the project's name: trimmed; blank clears the override
 * (`null`, back to the config/folder name); unchanged does nothing.
 */
export function commitProjectName(
  project: Pick<ProjectInfo, "id" | "name">,
  draft: string,
): void {
  const next = draft.trim();
  if (next === project.name) return;
  void useProjectStore
    .getState()
    .renameProject(project.id, next.length > 0 ? next : null);
}
