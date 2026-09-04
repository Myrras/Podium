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

import type { AutonomyMode, QueuedMessage } from "../ipc/types";
import type { ChatProposal } from "../state/assistantStore";
import { useAssistantStore } from "../state/assistantStore";
import { useLayoutStore } from "../state/layoutStore";
import { Markdown } from "./Markdown";
import {
  AssistantIcon,
  CloseIcon,
  DeleteIcon,
  EditIcon,
  MemoryIcon,
  SendIcon,
  StopIcon,
} from "./icons";
import styles from "./AssistantPane.module.css";

/** Shared with the Settings → Assistant tab, which mirrors this same picker. */
export const AUTONOMY_LABELS: Record<AutonomyMode, string> = {
  full: "Do everything itself",
  "confirm-destructive": "Ask before risky actions",
  "propose-all": "Ask before every action",
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
  const pending = useAssistantStore((s) => s.pending);
  const interrupt = useAssistantStore((s) => s.interrupt);
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

  // Follow the tail as turns stream in (waiting rows count too — they live
  // at the bottom of the same scroll container).
  useEffect(() => {
    const log = logRef.current;
    if (log) log.scrollTop = log.scrollHeight;
  }, [messages, busy, pending]);

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
        {messages.length === 0 && pending.length === 0 && (
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
            ) : m.role === "nudge" ? (
              // Podium spoke to the *assistant*, not the user: a discreet
              // marker, not the internal instruction text (hover for it).
              <span title={m.text}>Podium nudged the assistant</span>
            ) : (
              m.text
            )}
          </div>
        ))}
        {pending.map((p) => (
          <PendingRow key={p.id} item={p} />
        ))}
        {busy && (
          <div className={styles.working} role="status">
            <span>{activity ? `${actionLabel(activity)}…` : "thinking…"}</span>
            <span className={styles.workingDots} aria-hidden="true">
              <span />
              <span />
              <span />
            </span>
            <button
              type="button"
              className={styles.interruptBtn}
              aria-label="Interrupt"
              title="Stop this turn and send what's waiting"
              onClick={() => void interrupt()}
            >
              <StopIcon />
            </button>
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

/**
 * One message waiting for the current turn to end. Only a user message is
 * editable/removable — a nudge or proposal-outcome notice is Podium's own
 * text, shown as-is until it is delivered.
 */
function PendingRow({ item }: { item: QueuedMessage }) {
  const editPending = useAssistantStore((s) => s.editPending);
  const removePending = useAssistantStore((s) => s.removePending);
  const [draft, setDraft] = useState<string | null>(null);
  const editing = draft !== null;

  function save() {
    const text = (draft ?? "").trim();
    if (text && text !== item.text) void editPending(item.id, text);
    setDraft(null);
  }

  return (
    <div className={styles.pending} data-editing={editing}>
      {editing ? (
        <input
          className={styles.pendingInput}
          value={draft ?? ""}
          autoFocus
          aria-label="Edit waiting message"
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") save();
            if (e.key === "Escape") setDraft(null);
          }}
          onBlur={save}
        />
      ) : (
        <span className={styles.pendingText}>{item.text}</span>
      )}
      <span className={styles.pendingStatus}>waiting…</span>
      {item.kind === "user" && !editing && (
        <div className={styles.pendingActions}>
          <button
            type="button"
            className={styles.iconBtn}
            aria-label="Edit waiting message"
            title="Edit before it is sent"
            onClick={() => setDraft(item.text)}
          >
            <EditIcon />
          </button>
          <button
            type="button"
            className={styles.iconBtn}
            aria-label="Remove waiting message"
            title="Withdraw — it will never be sent"
            onClick={() => void removePending(item.id)}
          >
            <DeleteIcon />
          </button>
        </div>
      )}
    </div>
  );
}
