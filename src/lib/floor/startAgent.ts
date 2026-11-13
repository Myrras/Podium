import type { ProjectInfo } from "../../ipc/types";
import { useAssistantStore } from "../../state/assistantStore";
import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";
import { useProjectStore } from "../../state/projectStore";

/**
 * The + on a nameplate: drafts a prompt in the assistant when it is on and
 * its panel open, else (or with Cmd/Ctrl held) opens the New agent screen.
 */
export function startAgentIn(
  project: ProjectInfo,
  e: { metaKey: boolean; ctrlKey: boolean },
): void {
  useProjectStore.getState().setActiveProject(project.id);
  const draft =
    !e.metaKey &&
    !e.ctrlKey &&
    useAssistantStore.getState().enabled &&
    useLayoutStore.getState().assistantOpen;
  if (draft) {
    useAssistantStore
      .getState()
      .setDraft(`Start an agent in ${project.name} on `);
  } else {
    useFloorStore.getState().setAgentModal({ projectId: project.id });
  }
}
