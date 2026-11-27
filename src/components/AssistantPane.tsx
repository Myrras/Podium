/**
 * The personal assistant panel (Project mode) — a collapsible island on the
 * right of the app, workspace-wide (it is not tied to the selected project).
 *
 * A strip with the house bot and the chat's controls, the overview list of
 * what needs the user, the work the assistant handed to its own sub-agents,
 * then the conversation (`AssistantChat`). Resetting the conversation lives in
 * Settings → Assistant — day to day you stay in the one session. In Floor mode
 * the same list and chat open as popovers on the front desk instead.
 */

import { useLayoutStore } from "../state/layoutStore";
import { AssistantChat } from "./AssistantChat";
import { AssistantDesk, useAssistantDesk } from "./AssistantDesk";
import { ResponsesSection } from "./ResponsesSection";
import { SubagentsSection } from "./SubagentsSection";
import { CloseIcon } from "./icons";
import styles from "./AssistantPane.module.css";

/** Shared with the Settings → Assistant tab, which mirrors this same picker. */
export { AUTONOMY_LABELS } from "./AssistantChat";

export function AssistantPane() {
  const desk = useAssistantDesk();
  const setAssistantOpen = useLayoutStore((s) => s.setAssistantOpen);
  const width = useLayoutStore((s) => s.assistantWidth);

  return (
    <aside
      className={styles.pane}
      style={{ width, minWidth: width }}
      aria-label="Personal assistant"
    >
      <AssistantChat
        header={(controls) => (
          <header className={styles.header}>
            <div className={styles.stage}>
              <AssistantDesk state={desk.state} helpers={desk.helpers}>
                <span className={styles.title}>Assistant</span>
              </AssistantDesk>
            </div>
            {controls}
            <button
              type="button"
              className={styles.iconBtn}
              aria-label="Hide assistant"
              title="Hide assistant"
              onClick={() => setAssistantOpen(false)}
            >
              <CloseIcon />
            </button>
          </header>
        )}
      >
        <ResponsesSection />
        <SubagentsSection />
      </AssistantChat>
    </aside>
  );
}
