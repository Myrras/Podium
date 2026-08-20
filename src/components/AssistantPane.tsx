/**
 * The personal assistant chat panel — a collapsible island on the right of
 * the app, workspace-wide (it is not tied to the selected project).
 *
 * It renders the conversation — the user's turns, the assistant's prose
 * (markdown), the actions it ran, and failures — plus the actions its autonomy
 * mode held back, as cards the user confirms or rejects right here. The log is
 * replayed from the persisted transcript on mount; the header can open the
 * assistant's memory for editing (resetting the conversation lives in
 * Settings → Assistant — day to day you stay in the one session). Podium's own
 * proactive nudges (an agent finished, one looks stuck, a day-start summary)
 * arrive as notice rows, so an unprompted reply always has a visible reason.
 */

import { useEffect, useRef, useState } from "react";

import type { AutonomyMode } from "../ipc/types";
import type { ChatProposal } from "../state/assistantStore";
import { useAssistantStore } from "../state/assistantStore";
import { useLayoutStore } from "../state/layoutStore";
import { Markdown } from "./Markdown";
import { AssistantIcon, CloseIcon, MemoryIcon, SendIcon } from "./icons";
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
  const activity = useAssistantStore((s) => s.activity);
  const send = useAssistantStore((s) => s.send);
  const autonomy = useAssistantStore((s) => s.autonomy);
  const setAutonomy = useAssistantStore((s) => s.setAutonomy);
  const loadAutonomy = useAssistantStore((s) => s.loadAutonomy);
  const hydrate = useAssistantStore((s) => s.hydrate);
  const setAssistantOpen = useLayoutStore((s) => s.setAssistantOpen);
  const width = useLayoutStore((s) => s.assistantWidth);
  const [draft, setDraft] = useState("");
  const [memoryOpen, setMemoryOpen] = useState(false);
  const logRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // Both are persisted server-side; the store only mirrors them.
  useEffect(() => {
    void loadAutonomy();
    void hydrate();
  }, [loadAutonomy, hydrate]);

  // Follow the tail as turns stream in.
  useEffect(() => {
    const log = logRef.current;
    if (log) log.scrollTop = log.scrollHeight;
  }, [messages, busy]);

  // Grow the composer with the draft, up to the CSS max-height cap (where it
  // starts scrolling instead). Re-runs on every keystroke and on submit()
  // clearing the draft, so the box shrinks back too.
  useEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [draft]);

  async function submit() {
    const text = draft.trim();
    if (!text) return;
    // Clear optimistically: the store keeps the turn either way (a failure
    // lands in the log as a notice), so re-typing it would be noise.
    setDraft("");
    await send(text);
  }

  return (
    <aside
      className={styles.pane}
      style={{ width, minWidth: width }}
      aria-label="Personal assistant"
    >
      <header className={styles.header}>
        <AssistantIcon className={styles.headerIcon} />
        <span className={styles.title}>Assistant</span>
        <button
          type="button"
          className={styles.iconBtn}
          aria-label="Assistant memory"
          title="What the assistant remembers"
          aria-pressed={memoryOpen}
          onClick={() => setMemoryOpen(!memoryOpen)}
        >
          <MemoryIcon />
        </button>
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
          className={styles.iconBtn}
          aria-label="Hide assistant"
          title="Hide assistant"
          onClick={() => setAssistantOpen(false)}
        >
          <CloseIcon />
        </button>
      </header>

      {memoryOpen && <MemoryEditor onClose={() => setMemoryOpen(false)} />}

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
            <span>{activity ? `${actionLabel(activity)}…` : "thinking…"}</span>
            <span className={styles.workingDots} aria-hidden="true">
              <span />
              <span />
              <span />
            </span>
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
          ref={inputRef}
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

/**
 * The assistant's long-term memory, as plain text the user can rewrite or
 * clear. It is markdown the assistant maintains itself, so editing it by hand
 * is a correction — not the main way it grows.
 */
function MemoryEditor({ onClose }: { onClose: () => void }) {
  const memory = useAssistantStore((s) => s.memory);
  const loadMemory = useAssistantStore((s) => s.loadMemory);
  const saveMemory = useAssistantStore((s) => s.saveMemory);
  const [draft, setDraft] = useState<string | null>(null);

  useEffect(() => {
    void loadMemory();
  }, [loadMemory]);

  // Start from whatever is stored, then let the user's edits win.
  const value = draft ?? memory ?? "";

  return (
    <section className={styles.memory} aria-label="Assistant memory">
      <textarea
        className={styles.memoryText}
        rows={8}
        value={value}
        placeholder="The assistant has not written anything down yet."
        aria-label="Memory"
        onChange={(e) => setDraft(e.target.value)}
      />
      <div className={styles.memoryActions}>
        <button
          type="button"
          className={styles.confirmBtn}
          onClick={async () => {
            await saveMemory(value);
            setDraft(null);
            onClose();
          }}
        >
          Save
        </button>
        <button
          type="button"
          className={styles.rejectBtn}
          onClick={() => setDraft("")}
        >
          Clear
        </button>
        <button
          type="button"
          className={styles.rejectBtn}
          onClick={() => {
            setDraft(null);
            onClose();
          }}
        >
          Close
        </button>
      </div>
    </section>
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
