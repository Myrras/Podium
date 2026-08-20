/**
 * The personal assistant chat panel — a collapsible island on the right of
 * the app, workspace-wide (it is not tied to the selected project).
 *
 * It renders the conversation — the user's turns, the assistant's prose
 * (markdown), the actions it ran, and failures — plus the actions its autonomy
 * mode held back, as cards the user confirms or rejects right here.
 * Persistence and proactive nudges come later.
 */

import { useEffect, useRef, useState } from "react";

import type { AutonomyMode } from "../ipc/types";
import type { ChatProposal } from "../state/assistantStore";
import { useAssistantStore } from "../state/assistantStore";
import { useLayoutStore } from "../state/layoutStore";
import { Markdown } from "./Markdown";
import { AssistantIcon, CloseIcon, SendIcon } from "./icons";
import styles from "./AssistantPane.module.css";

const AUTONOMY_LABELS: Record<AutonomyMode, string> = {
  full: "Act on its own",
  "confirm-destructive": "Confirm destructive",
  "propose-all": "Propose everything",
};

/** `set_todo_archived` → `set todo archived`, for the action cards. */
function actionLabel(tool: string): string {
  return tool.replace(/^mcp__[^_]+__/, "").replace(/_/g, " ");
}

export function AssistantPane() {
  const messages = useAssistantStore((s) => s.messages);
  const busy = useAssistantStore((s) => s.busy);
  const send = useAssistantStore((s) => s.send);
  const autonomy = useAssistantStore((s) => s.autonomy);
  const setAutonomy = useAssistantStore((s) => s.setAutonomy);
  const loadAutonomy = useAssistantStore((s) => s.loadAutonomy);
  const setAssistantOpen = useLayoutStore((s) => s.setAssistantOpen);
  const [draft, setDraft] = useState("");
  const logRef = useRef<HTMLDivElement>(null);

  // The mode is persisted server-side; the store only mirrors it.
  useEffect(() => {
    void loadAutonomy();
  }, [loadAutonomy]);

  // Follow the tail as turns stream in.
  useEffect(() => {
    const log = logRef.current;
    if (log) log.scrollTop = log.scrollHeight;
  }, [messages, busy]);

  async function submit() {
    const text = draft.trim();
    if (!text) return;
    // Clear optimistically: the store keeps the turn either way (a failure
    // lands in the log as a notice), so re-typing it would be noise.
    setDraft("");
    await send(text);
  }

  return (
    <aside className={styles.pane} aria-label="Personal assistant">
      <header className={styles.header}>
        <AssistantIcon className={styles.headerIcon} />
        <span className={styles.title}>Assistant</span>
        <select
          className={styles.autonomy}
          aria-label="Autonomy"
          title="How much the assistant may do without asking"
          value={autonomy}
          onChange={(e) => void setAutonomy(e.target.value as AutonomyMode)}
        >
          {Object.entries(AUTONOMY_LABELS).map(([mode, label]) => (
            <option key={mode} value={mode}>
              {label}
            </option>
          ))}
        </select>
        <button
          type="button"
          className={styles.closeBtn}
          aria-label="Hide assistant"
          title="Hide assistant"
          onClick={() => setAssistantOpen(false)}
        >
          <CloseIcon />
        </button>
      </header>

      <div className={styles.log} ref={logRef}>
        {messages.length === 0 && (
          <p className={styles.empty}>
            Ask for anything across your projects — to-dos, agents, an overview.
          </p>
        )}
        {messages.map((m) => (
          <div key={m.id} className={styles.message} data-role={m.role}>
            {m.role === "assistant" ? (
              <Markdown>{m.text}</Markdown>
            ) : m.role === "tool" ? (
              actionLabel(m.text)
            ) : m.proposal ? (
              <ProposalCard proposal={m.proposal} />
            ) : (
              m.text
            )}
          </div>
        ))}
        {busy && (
          <div className={styles.working} role="status">
            working…
          </div>
        )}
      </div>

      <form
        className={styles.composer}
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <textarea
          className={styles.input}
          value={draft}
          rows={2}
          placeholder="Message the assistant…"
          aria-label="Message the assistant"
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            // Enter sends, Shift+Enter is a newline (chat convention).
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void submit();
            }
          }}
        />
        <button
          type="submit"
          className={styles.sendBtn}
          aria-label="Send"
          title="Send"
          disabled={!draft.trim()}
        >
          <SendIcon />
        </button>
      </form>
    </aside>
  );
}

/** One held-back action: what it is, and the two buttons that settle it. */
function ProposalCard({ proposal }: { proposal: ChatProposal }) {
  const resolveProposal = useAssistantStore((s) => s.resolveProposal);
  const { id, tool, summary, status, error } = proposal;

  return (
    <div className={styles.proposal} data-status={status}>
      <span className={styles.proposalTool}>{actionLabel(tool)}</span>
      {summary && <code className={styles.proposalArgs}>{summary}</code>}
      {status === "pending" ? (
        <div className={styles.proposalActions}>
          <button
            type="button"
            className={styles.confirmBtn}
            onClick={() => void resolveProposal(id, true)}
          >
            Confirm
          </button>
          <button
            type="button"
            className={styles.rejectBtn}
            onClick={() => void resolveProposal(id, false)}
          >
            Reject
          </button>
        </div>
      ) : (
        <span className={styles.proposalStatus}>
          {status === "rejected"
            ? "Rejected"
            : status === "failed"
              ? `Failed: ${error ?? "unknown reason"}`
              : "Done"}
        </span>
      )}
    </div>
  );
}
