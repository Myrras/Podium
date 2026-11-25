/**
 * The front desk's popovers, above the desk: the overview list (count chip)
 * and the assistant chat (house bot). Anchored on `data-desk="<chat|list>"`;
 * closes when the anchor is gone (the Git board replaced the map).
 */

import { useEffect } from "react";

import { useFloorStore } from "../../state/floorStore";
import { AssistantChat } from "../AssistantChat";
import { deskLine, useAssistantDesk } from "../AssistantDesk";
import { Popover, PopoverSlot } from "../Popover";
import { ResponsesSection } from "../ResponsesSection";
import styles from "./FrontDeskPopovers.module.css";

export function FrontDeskPopovers() {
  const desk = useFloorStore((s) => s.deskPopover);
  const closeDesk = useFloorStore((s) => s.closeDesk);
  const anchor = desk
    ? document.querySelector<HTMLElement>(`[data-desk="${desk}"]`)
    : null;

  useEffect(() => {
    if (desk && !anchor) closeDesk();
  }, [desk, anchor, closeDesk]);

  if (!desk || !anchor) return null;
  return desk === "list" ? (
    <Popover
      key="list"
      anchor={anchor}
      title="What needs you"
      width={380}
      flush
      onClose={closeDesk}
    >
      {/* As in the pane: selecting text here is reading, not picking context. */}
      <div className={styles.list} data-assistant-pane="true">
        <ResponsesSection />
      </div>
    </Popover>
  ) : (
    <ChatPopover anchor={anchor} onClose={closeDesk} />
  );
}

function ChatPopover({
  anchor,
  onClose,
}: {
  anchor: HTMLElement;
  onClose: () => void;
}) {
  const { state, helpers } = useAssistantDesk();
  return (
    <Popover
      key="chat"
      anchor={anchor}
      title={
        <>
          Assistant
          <span className={styles.line} data-state={state}>
            {deskLine(state, helpers)}
          </span>
        </>
      }
      width={420}
      flush
      onClose={onClose}
    >
      <AssistantChat
        className={styles.chat}
        header={(controls) => (
          <PopoverSlot name="header">{controls}</PopoverSlot>
        )}
      />
    </Popover>
  );
}
