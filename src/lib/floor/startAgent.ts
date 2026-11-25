import type { ProjectInfo } from "../../ipc/types";
import { useAssistantStore } from "../../state/assistantStore";
import { useFloorStore } from "../../state/floorStore";
import { useProjectStore } from "../../state/projectStore";

/**
 * The + on a nameplate: drafts a prompt in the front desk's chat (opening
 * it) when the assistant is on, else (or with Cmd/Ctrl held) opens the New
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
    useAssistantStore
      .getState()
      .setDraft(`Start an agent in ${project.name} on `);
    useFloorStore.getState().openDesk("chat");
  } else {
    useFloorStore.getState().setAgentModal({ projectId: project.id });
  }
}
