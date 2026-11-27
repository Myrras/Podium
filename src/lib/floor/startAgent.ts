import type { ProjectInfo } from "../../ipc/types";
import { useAssistantStore } from "../../state/assistantStore";
import { useFloorStore } from "../../state/floorStore";
import { useProjectStore } from "../../state/projectStore";
import { addSelectedItems } from "../selection";

/**
 * The + on a nameplate: adds the project as assistant context and opens the
 * front desk's chat when the assistant is on, else (or with Cmd/Ctrl held) opens the New
 * agent screen.
 */
export function startAgentIn(
  project: ProjectInfo,
  e: { metaKey: boolean; ctrlKey: boolean },
): void {
  useProjectStore.getState().setActiveProject(project.id);
  const draft =
    !e.metaKey && !e.ctrlKey && useAssistantStore.getState().enabled;
  if (draft) {
    addSelectedItems([
      { kind: "project", projectId: project.id, id: project.id },
    ]);
    useAssistantStore.getState().requestFocus();
    useFloorStore.getState().openDesk("chat");
  } else {
    useFloorStore.getState().setAgentModal({ projectId: project.id });
  }
}
