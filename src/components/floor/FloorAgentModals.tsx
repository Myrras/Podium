/** The New agent screens of Floor mode, opened through floorStore. */

import { useFloorStore } from "../../state/floorStore";
import { NewAgentModal } from "../NewAgentModal";
import { ScratchpadAgentModal } from "../ScratchpadAgentModal";

export function FloorAgentModals() {
  const agentModal = useFloorStore((s) => s.agentModal);
  const setAgentModal = useFloorStore((s) => s.setAgentModal);
  const scratchpadAgentModal = useFloorStore((s) => s.scratchpadAgentModal);
  const setScratchpadAgentModal = useFloorStore(
    (s) => s.setScratchpadAgentModal,
  );

  return (
    <>
      <NewAgentModal
        open={agentModal !== null}
        projectId={agentModal?.projectId ?? null}
        todoIds={agentModal?.todoIds}
        initialName={agentModal?.initialName}
        onClose={() => setAgentModal(null)}
      />
      <ScratchpadAgentModal
        open={scratchpadAgentModal !== null}
        projectId={scratchpadAgentModal?.projectId ?? null}
        scratchpadIds={scratchpadAgentModal?.scratchpadIds ?? []}
        initialName={scratchpadAgentModal?.initialName}
        onClose={() => setScratchpadAgentModal(null)}
      />
    </>
  );
}
