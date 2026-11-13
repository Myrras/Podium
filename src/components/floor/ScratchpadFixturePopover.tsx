/** The scratchpad shelf's popover: the list, and one scratchpad opened inside it. */
import type { ProjectId } from "../../ipc/types";
import { useFloorStore } from "../../state/floorStore";
import { useProjectStore } from "../../state/projectStore";
import { useScratchpadStore } from "../../state/scratchpadStore";
import { Popover } from "../Popover";
import { ScratchpadDetailPane } from "../ScratchpadDetailPane";
import { ScratchpadSubsection } from "../ScratchpadSubsection";
import styles from "./ScratchpadFixturePopover.module.css";

export function ScratchpadFixturePopover({
  projectId,
  itemId,
  anchor,
}: {
  projectId: ProjectId;
  itemId: string | null;
  anchor: HTMLElement | null;
}) {
  const root = useProjectStore(
    (s) => s.projects.find((p) => p.id === projectId)?.root ?? "",
  );
  const title = useScratchpadStore((s) =>
    itemId
      ? s.scratchpadsByProject[projectId]?.find((p) => p.id === itemId)?.title
      : undefined,
  );
  const { closePopover, setPopoverItem, setScratchpadAgentModal } =
    useFloorStore.getState();

  return (
    <Popover
      anchor={anchor}
      title="Scratchpads"
      width={itemId ? 560 : 360}
      onClose={closePopover}
      onBack={itemId ? () => setPopoverItem(null) : undefined}
      backLabel="Back to scratchpads"
    >
      {itemId ? (
        <>
          <ScratchpadDetailPane
            key={itemId}
            compact
            projectId={projectId}
            scratchpadId={itemId}
            onClose={() => setPopoverItem(null)}
          />
          {title !== undefined && (
            <div className={styles.agentRow}>
              <button
                type="button"
                onClick={() =>
                  setScratchpadAgentModal({
                    projectId,
                    scratchpadIds: [itemId],
                    initialName: title,
                  })
                }
              >
                Start agent on this
              </button>
            </div>
          )}
        </>
      ) : (
        <ScratchpadSubsection
          bare
          projectId={projectId}
          projectRoot={root}
          onOpenScratchpad={(_, id) => setPopoverItem(id)}
          onPickAgent={(pid, scratchpadIds, initialName) =>
            setScratchpadAgentModal({
              projectId: pid,
              scratchpadIds,
              initialName,
            })
          }
        />
      )}
    </Popover>
  );
}
