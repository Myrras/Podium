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
 * The work it handed to its own sub-agents is listed above the log.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

import type { Attachment, AutonomyMode, QueuedMessage } from "../ipc/types";
import {
  contextAttachment,
  useAssistantContext,
} from "../lib/assistantContext";
import { textPreview } from "../lib/textPreview";
import type { ChatMessage, ChatProposal } from "../state/assistantStore";
import { useAssistantStore } from "../state/assistantStore";
import { useLayoutStore } from "../state/layoutStore";
import { Markdown } from "./Markdown";
import { ResponsesSection } from "./ResponsesSection";
import { SubagentsSection } from "./SubagentsSection";
import {
  AssistantIcon,
  AttachIcon,
  CloseIcon,
  DeleteIcon,
  EditIcon,
  FileAttachmentIcon,
  ImageAttachmentIcon,
  MemoryIcon,
  ReplyIcon,
  SendIcon,
  StopIcon,
} from "./icons";
import styles from "./AssistantPane.module.css";

/** Most attachments one message may carry — mirrors the backend's own cap. */
const MAX_ATTACHMENTS = 5;
/** Largest a text fallback may be, client-side — mirrors the backend's cap. */
const MAX_INLINE_TEXT_BYTES = 100_000;
/** Extensions read from disk (picker/drop path) or shown with an image icon. */
const IMAGE_EXTENSIONS = new Set(["png", "jpg", "jpeg", "gif", "webp"]);
/** Extensions/mime types a dropped or pasted `File` (no path, no bytes on
 * disk otherwise) is still worth inlining as text rather than refusing. */
const TEXT_EXTENSIONS = new Set([
  "txt",
  "md",
  "log",
  "json",
  "yaml",
  "yml",
  "toml",
  "csv",
  "ts",
  "tsx",
  "js",
  "jsx",
  "rs",
  "py",
  "go",
  "java",
  "c",
  "h",
  "cpp",
  "hpp",
  "css",
  "html",
  "xml",
  "sh",
  "conf",
  "ini",
]);

/** An attachment still in the composer, not yet sent. */
interface PendingAttachment {
  id: string;
  kind: "image" | "path" | "text";
  name: string;
  /** image only */
  mediaType?: string;
  /** image only, base64 (no `data:` prefix) */
  data?: string;
  /** path only, absolute */
  path?: string;
  /** text only */
  text?: string;
}

function toAttachment(a: PendingAttachment): Attachment {
  switch (a.kind) {
    case "image":
      return {
        kind: "image",
        name: a.name,
        mediaType: a.mediaType!,
        data: a.data!,
      };
    case "path":
      return { kind: "path", path: a.path! };
    case "text":
      return { kind: "text", name: a.name, text: a.text! };
  }
}

function fileName(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

function isImagePath(path: string): boolean {
  const ext = path.split(".").pop()?.toLowerCase();
  return !!ext && IMAGE_EXTENSIONS.has(ext);
}

function looksLikeText(file: File): boolean {
  if (file.type.startsWith("text/") || file.type === "application/json") {
    return true;
  }
  const ext = file.name.split(".").pop()?.toLowerCase();
  return !!ext && TEXT_EXTENSIONS.has(ext);
}

/** Base64 (no `data:` prefix) of a `File`'s bytes. */
function readAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = reader.result as string;
      resolve(result.slice(result.indexOf(",") + 1));
    };
    reader.onerror = () => reject(reader.error as Error);
    reader.readAsDataURL(file);
  });
}

/**
 * Classify dropped or pasted `File`s (real bytes, but — unlike the picker —
 * never a real filesystem path: the webview's drag-drop stays on the
 * standard HTML5 API precisely so the sidebar's own drag-to-reorder keeps
 * working) into attachments. An image inlines directly; a text-looking file
 * inlines as its own fallback; anything else has no path to hand over and no
 * safe way to inline it, so it is refused by name.
 */
async function attachmentsFromFiles(
  files: File[],
): Promise<{ attachments: PendingAttachment[]; refused: string[] }> {
  const attachments: PendingAttachment[] = [];
  const refused: string[] = [];
  for (const file of files) {
    if (file.type.startsWith("image/")) {
      attachments.push({
        id: crypto.randomUUID(),
        kind: "image",
        name: file.name || "pasted-image.png",
        mediaType: file.type,
        data: await readAsBase64(file),
      });
    } else if (looksLikeText(file) && file.size <= MAX_INLINE_TEXT_BYTES) {
      attachments.push({
        id: crypto.randomUUID(),
        kind: "text",
        name: file.name || "pasted-text.txt",
        text: await file.text(),
      });
    } else {
      refused.push(file.name || "attachment");
    }
  }
  return { attachments, refused };
}

/** Shared with the Settings → Assistant tab, which mirrors this same picker. */
export const AUTONOMY_LABELS: Record<AutonomyMode, string> = {
  full: "Do everything itself",
  "confirm-destructive": "Ask before risky actions",
  "propose-all": "Ask before every action",
};

/**
 * `mcp__claude_ai_Slack__slack_read_channel` → `{ server: "Slack", tool:
 * "slack read channel" }`. Server names hold only single `_`, but tool names
 * may hold `__`, so the split is on the first `__`. Non-MCP names (`Agent`, `set_todo_archived`) have no
 * server.
 */
function parseAction(name: string): { server?: string; tool: string } {
  const rest = name.startsWith("mcp__") ? name.slice(5) : null;
  const at = rest?.indexOf("__") ?? -1;
  if (rest === null || at < 0) return { tool: name.replace(/_/g, " ") };
  const server = rest.slice(0, at).replace(/^claude_ai_/, "");
  return {
    server: server === "podium" ? "Podium" : server.replace(/_/g, " "),
    tool: rest.slice(at + 2).replace(/_+/g, " "),
  };
}

/** `Slack · slack read channel`, or just the tool for a non-MCP action. */
function actionLabel(name: string): string {
  const { server, tool } = parseAction(name);
  return server ? `${server} · ${tool}` : tool;
}

/** Distinct servers named in a run's summary before it says `+k`. */
const SUMMARY_SERVERS = 3;

/** How far from the bottom still counts as "at the bottom" when following. */
const AT_BOTTOM_PX = 40;

/** Rows that fold into one line: actions, their failures, Podium's nudges. */
function isActivity(m: ChatMessage): boolean {
  return m.role === "tool" || m.role === "toolFailed" || m.role === "nudge";
}

/**
 * True when the caret has no newline before it — i.e. it's on the textarea's
 * first *text* line. (jsdom has no layout, so this is defined by literal
 * `\n`s, not by where a long line visually wraps — the same limit any
 * caller faces without a real renderer, and good enough for a composer.)
 */
function isCaretOnFirstLine(el: HTMLTextAreaElement): boolean {
  return !el.value.slice(0, el.selectionStart ?? 0).includes("\n");
}

/** True when the caret has no newline after it — the textarea's last line. */
function isCaretOnLastLine(el: HTMLTextAreaElement): boolean {
  return !el.value.slice(el.selectionEnd ?? el.value.length).includes("\n");
}

export function AssistantPane() {
  const messages = useAssistantStore((s) => s.messages);
  // Regrouped only when the transcript changes, not on composer keystrokes.
  const groups = useMemo(() => groupActivity(messages), [messages]);
  const busy = useAssistantStore((s) => s.busy);
  const activity = useAssistantStore((s) => s.activity);
  const send = useAssistantStore((s) => s.send);
  const pushNotice = useAssistantStore((s) => s.pushNotice);
  const pending = useAssistantStore((s) => s.pending);
  const interrupt = useAssistantStore((s) => s.interrupt);
  const autonomy = useAssistantStore((s) => s.autonomy);
  const setAutonomy = useAssistantStore((s) => s.setAutonomy);
  const loadAutonomy = useAssistantStore((s) => s.loadAutonomy);
  const enabled = useAssistantStore((s) => s.enabled);
  const setEnabled = useAssistantStore((s) => s.setEnabled);
  const hydrate = useAssistantStore((s) => s.hydrate);
  const dayStart = useAssistantStore((s) => s.dayStart);
  const loadResponses = useAssistantStore((s) => s.loadResponses);
  const setAssistantOpen = useLayoutStore((s) => s.setAssistantOpen);
  const width = useLayoutStore((s) => s.assistantWidth);
  const [draft, setDraft] = useState("");
  const [attachments, setAttachments] = useState<PendingAttachment[]>([]);
  const [dragOver, setDragOver] = useState(false);
  // What the user has selected elsewhere in Podium, shown as chips and
  // sent along as context. Dismissals are by key, so a chip stays gone
  // until the selection itself changes.
  const context = useAssistantContext();
  const [dismissed, setDismissed] = useState<string[]>([]);
  const activeContext = context.filter((c) => !dismissed.includes(c.key));
  const [memoryOpen, setMemoryOpen] = useState(false);
  const logRef = useRef<HTMLDivElement>(null);
  // Whether the log is scrolled to the bottom, so streaming only drags the
  // view along when the user is not reading further up. A ref (written by
  // onScroll) rather than state: nothing renders from it.
  const atBottomRef = useRef(true);
  const [newActivity, setNewActivity] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  // Arrow-up/down history recall: an index into `history` (newest last),
  // `null` while not recalling. `draftBeforeRecall` is what the user had
  // typed before the first arrow-up, restored when walking past the newest
  // entry again — recalling never sends anything and never touches the
  // stored transcript, it only reads the `user` rows already in `messages`.
  const [historyIndex, setHistoryIndex] = useState<number | null>(null);
  const [draftBeforeRecall, setDraftBeforeRecall] = useState("");
  const history = messages.filter((m) => m.role === "user").map((m) => m.text);

  // Both are persisted server-side; the store only mirrors them.
  useEffect(() => {
    void loadAutonomy();
    void hydrate();
    void loadResponses();
  }, [loadAutonomy, hydrate, loadResponses]);

  function scrollToBottom() {
    const log = logRef.current;
    if (log) log.scrollTop = log.scrollHeight;
    atBottomRef.current = true;
    setNewActivity(false);
  }

  // Follow the tail as turns stream in (waiting rows count too — they live
  // at the bottom of the same scroll container) — but only from the bottom: a
  // user who scrolled up to read gets a pill instead of being yanked down —
  // only when a row worth reading was appended (prose, a notice, a proposal;
  // more actions or nudges folding into runs do not warrant it) and never on
  // busy/queue changes. A shrinking transcript (new conversation) clears it.
  // Sending jumps because submit() scrolls down first.
  const prevCount = useRef(messages.length);
  useEffect(() => {
    const prev = prevCount.current;
    prevCount.current = messages.length;
    const news = messages.slice(prev).some((m) => !isActivity(m));
    if (atBottomRef.current) scrollToBottom();
    // A new conversation: re-pin too, since no scroll event may follow.
    else if (messages.length < prev) scrollToBottom();
    else if (news) setNewActivity(true);
  }, [messages, busy, pending]);

  function syncAtBottom() {
    const el = logRef.current;
    if (!el) return;
    atBottomRef.current =
      el.scrollHeight - el.scrollTop - el.clientHeight < AT_BOTTOM_PX;
    if (atBottomRef.current) setNewActivity(false);
  }

  // Opening or closing a folded run changes the content height without a
  // scroll event. A reader pinned to the bottom stays there; anyone else is
  // left where they are (re-measuring here would drop the follow whenever the
  // expanded list grows below the viewport). Only the last run can stay
  // pinned: scrolling down for an earlier one would push the run the user just
  // opened out of view, so that one just re-measures.
  function onRunToggle(isLast: boolean) {
    if (isLast && atBottomRef.current) scrollToBottom();
    else if (!isLast) syncAtBottom();
  }

  // Grow the composer with the draft, up to the CSS max-height cap (where it
  // starts scrolling instead). Re-runs on every keystroke and on submit()
  // clearing the draft, so the box shrinks back too.
  useEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [draft]);

  /** Add attachments, refusing (with a notice naming each) whatever would
   * push the tray past the per-message cap — the same limit the backend
   * enforces, checked here too for immediate feedback. */
  function addAttachments(items: PendingAttachment[]) {
    setAttachments((current) => {
      const room = Math.max(0, MAX_ATTACHMENTS - current.length);
      items.slice(room).forEach((item) => {
        pushNotice(
          `"${item.name}" was not added: at most ${MAX_ATTACHMENTS} attachments per message.`,
        );
      });
      return [...current, ...items.slice(0, room)];
    });
  }

  function removeAttachment(id: string) {
    setAttachments((current) => current.filter((a) => a.id !== id));
  }

  async function pickFiles() {
    const selected = await openFileDialog({ multiple: true });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    addAttachments(
      paths.map((path) => ({
        id: crypto.randomUUID(),
        kind: "path" as const,
        name: fileName(path),
        path,
      })),
    );
  }

  /**
   * Take dropped or pasted `File`s into the tray. Unlike the picker these
   * carry bytes but no filesystem path, so whatever can be neither inlined
   * as an image nor as text is reported by name instead of vanishing.
   */
  async function addFiles(files: File[]) {
    if (files.length === 0) return;
    const { attachments: added, refused } = await attachmentsFromFiles(files);
    for (const name of refused) {
      pushNotice(
        `"${name}" was not added: only images and text files can be attached this way — drop it from Finder, or use the paperclip to pick it by path.`,
      );
    }
    if (added.length > 0) addAttachments(added);
  }

  async function submit() {
    const text = draft.trim();
    if (!text && attachments.length === 0) return;
    // ponytail: the backend refuses a blank turn outright, so an
    // attachment-only send needs some caption — add a real one (voice
    // memo / smarter default) if users actually send captionless attachments.
    const caption = text || "(sent without a message)";
    // Clear optimistically: the store keeps the turn either way (a failure
    // lands in the log as a notice), so re-typing it would be noise.
    setDraft("");
    setHistoryIndex(null);
    // Directly, not just the ref: a momentum scroll event could reset the ref
    // before the new row arrives.
    scrollToBottom();
    const toSend = attachments;
    setAttachments([]);
    // Files the user picked come first: with both past the backend's
    // per-message cap, the explicit attachment outranks the ambient context.
    await send(caption, [
      ...toSend.map(toAttachment),
      ...activeContext.map(contextAttachment),
    ]);
  }

  /**
   * Walk the composer's history: `-1` for arrow-up (older), `1` for
   * arrow-down (newer/back-to-draft). Only fires when the caret is on the
   * edge line arrow-up/down would otherwise leave — checked by the caller —
   * so it never steals the cursor movement a multi-line draft needs.
   */
  function recall(direction: -1 | 1) {
    if (direction === -1) {
      if (history.length === 0) return;
      const idx = historyIndex === null ? history.length - 1 : historyIndex - 1;
      if (idx < 0) return;
      if (historyIndex === null) setDraftBeforeRecall(draft);
      setHistoryIndex(idx);
      setDraft(history[idx]);
    } else {
      if (historyIndex === null) return;
      if (historyIndex + 1 < history.length) {
        setHistoryIndex(historyIndex + 1);
        setDraft(history[historyIndex + 1]);
      } else {
        setHistoryIndex(null);
        setDraft(draftBeforeRecall);
      }
    }
  }

  return (
    <aside
      className={styles.pane}
      style={{ width, minWidth: width }}
      aria-label="Personal assistant"
      data-assistant-pane="true"
      data-dragover={dragOver || undefined}
      onDragOver={(e) => {
        // The webview's own drag-drop is off (`dragDropEnabled: false`), so
        // the standard HTML5 events are ours to handle.
        e.preventDefault();
        setDragOver(true);
      }}
      onDragLeave={(e) => {
        // Only when the pointer actually left the pane, not on every child
        // boundary crossed on the way in.
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
          setDragOver(false);
        }
      }}
      onDrop={(e) => {
        e.preventDefault();
        setDragOver(false);
        void addFiles(Array.from(e.dataTransfer.files));
      }}
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

      {!enabled ? (
        <div className={styles.offState}>
          <p className={styles.offTitle}>The assistant is off</p>
          <p className={styles.empty}>
            It will not run, nudge you or send notifications.
          </p>
          <button
            type="button"
            className={styles.confirmBtn}
            onClick={() => void setEnabled(true)}
          >
            Turn on
          </button>
        </div>
      ) : (
        <>
      {memoryOpen && <MemoryEditor onClose={() => setMemoryOpen(false)} />}

      <ResponsesSection />

      <SubagentsSection />

      <div className={styles.log} ref={logRef} onScroll={syncAtBottom}>
        {!messages.some((m) => !isActivity(m)) && pending.length === 0 && (
          <div className={styles.emptyState}>
            <p className={styles.empty}>
              Ask for anything across your projects — to-dos, agents, an
              overview.
            </p>
            <button
              type="button"
              className={styles.confirmBtn}
              aria-label="Start my day"
              title="Get a summary of what's open across your projects"
              onClick={() => {
                scrollToBottom();
                void dayStart();
              }}
            >
              Start my day
            </button>
          </div>
        )}
        {groups.map((g, i) =>
          Array.isArray(g) ? (
            <ActivityRun
              key={g[0].id}
              rows={g}
              onToggle={() => onRunToggle(i === groups.length - 1)}
            />
          ) : (
            <div key={g.id} className={styles.message} data-role={g.role}>
              {g.role === "assistant" ? (
                <Markdown>{g.text}</Markdown>
              ) : g.proposal ? (
                <ProposalCard proposal={g.proposal} />
              ) : g.role === "reply" ? (
                <ReplyLine text={g.text} />
              ) : (
                g.text
              )}
            </div>
          ),
        )}
        {/* Pinned to the bottom of the log: what the turn is doing, and what
            is queued behind it. A message the turn has picked up is gone from
            `pending` by then (the backend's queue snapshot), so nothing
            already in flight lingers here. */}
        {(busy || pending.length > 0) && (
          <div className={styles.tail}>
            {busy && (
              <div className={styles.working} role="status">
                <span>
                  {activity ? `${actionLabel(activity)}…` : "thinking…"}
                </span>
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
            {pending.map((p) => (
              <PendingRow key={p.id} item={p} />
            ))}
          </div>
        )}
      </div>

      {newActivity && (
        <button
          type="button"
          className={styles.newActivity}
          onClick={scrollToBottom}
        >
          New activity ↓
        </button>
      )}

      <form
        className={styles.composer}
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        {activeContext.length > 0 && (
          <ul className={styles.tray} aria-label="Context">
            {activeContext.map((c) => (
              <li key={c.key} className={styles.contextChip}>
                <span className={styles.chipName} title={c.text}>
                  {c.label}
                </span>
                <button
                  type="button"
                  className={styles.iconBtn}
                  aria-label={`Remove ${c.label} from context`}
                  title="Don't send this as context"
                  onClick={() => setDismissed((d) => [...d, c.key])}
                >
                  <CloseIcon />
                </button>
              </li>
            ))}
          </ul>
        )}

        {attachments.length > 0 && (
          <ul className={styles.tray} aria-label="Attachments">
            {attachments.map((a) => (
              <li key={a.id} className={styles.chip}>
                {a.kind === "image" || (a.path && isImagePath(a.path)) ? (
                  <ImageAttachmentIcon />
                ) : (
                  <FileAttachmentIcon />
                )}
                <span className={styles.chipName} title={a.path ?? a.name}>
                  {a.name}
                </span>
                <button
                  type="button"
                  className={styles.iconBtn}
                  aria-label={`Remove ${a.name}`}
                  title="Remove before sending"
                  onClick={() => removeAttachment(a.id)}
                >
                  <CloseIcon />
                </button>
              </li>
            ))}
          </ul>
        )}

        <div className={styles.composerRow}>
          <button
            type="button"
            className={styles.iconBtn}
            aria-label="Attach files"
            title="Attach images or files"
            onClick={() => void pickFiles()}
          >
            <AttachIcon />
          </button>
          <textarea
            ref={inputRef}
            className={styles.input}
            value={draft}
            rows={2}
            placeholder="Message the assistant…"
            aria-label="Message the assistant"
            onChange={(e) => setDraft(e.target.value)}
            onPaste={(e) => {
              // Only intercept a paste that carries files (a screenshot, a
              // copied file); plain text keeps the browser's own behaviour.
              const files = Array.from(e.clipboardData.files);
              if (files.length === 0) return;
              e.preventDefault();
              void addFiles(files);
            }}
            onKeyDown={(e) => {
              // Enter sends, Shift+Enter is a newline (chat convention).
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void submit();
                return;
              }
              // Recall only from the edge line the arrow would otherwise
              // leave the composer through — anywhere else the arrow just
              // moves the cursor, as it must in a multi-line draft.
              if (e.key === "ArrowUp" && isCaretOnFirstLine(e.currentTarget)) {
                if (history.length === 0) return;
                e.preventDefault();
                recall(-1);
              } else if (
                e.key === "ArrowDown" &&
                isCaretOnLastLine(e.currentTarget) &&
                historyIndex !== null
              ) {
                e.preventDefault();
                recall(1);
              }
            }}
          />
          <button
            type="submit"
            className={styles.sendBtn}
            aria-label="Send"
            title="Send"
            disabled={!draft.trim() && attachments.length === 0}
          >
            <SendIcon />
          </button>
        </div>
      </form>
        </>
      )}
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
 * Consecutive activity rows become one array; everything else stays a row.
 * A nudge starts a new run unless the current one already holds one.
 */
function groupActivity(
  messages: ChatMessage[],
): (ChatMessage | ChatMessage[])[] {
  const out: (ChatMessage | ChatMessage[])[] = [];
  for (const m of messages) {
    const prev = out[out.length - 1];
    if (!isActivity(m)) out.push(m);
    // A nudge opens its own run unless the current one already holds a nudge:
    // consecutive Podium checks merge, but a user turn's actions are never
    // absorbed into (and relabelled as) Podium's.
    else if (
      Array.isArray(prev) &&
      (m.role !== "nudge" || prev.some((r) => r.role === "nudge"))
    )
      prev.push(m);
    else out.push([m]);
  }
  return out;
}

/**
 * A run of actions (and the failures and nudges among them) as one collapsed
 * line: the log stays readable when a turn fires dozens of tool calls. The
 * summary names the first few servers; opening it lists each distinct action
 * once with a count. A failure's error shows clipped, with the full text on hover,
 * since it is usually something the assistant already dealt with. A run holding a nudge
 * is Podium's own check, so it reads fainter than the assistant's actions.
 */
function ActivityRun({
  rows,
  onToggle,
}: {
  rows: ChatMessage[];
  onToggle: () => void;
}) {
  const tools = rows.filter((m) => m.role === "tool");
  const fails = rows.filter((m) => m.role === "toolFailed");
  const nudges = rows.filter((m) => m.role === "nudge");

  const counts = new Map<string, number>();
  for (const m of tools) {
    const label = actionLabel(m.text);
    counts.set(label, (counts.get(label) ?? 0) + 1);
  }
  // Server (or bare tool name) of each distinct one, in first-seen order.
  const groups = [
    ...new Set(
      tools.map((m) => {
        const { server, tool } = parseAction(m.text);
        return server ?? tool;
      }),
    ),
  ];

  const times = (n: number) => (n > 1 ? ` ×${n}` : "");
  const onlyNudges = tools.length === 0 && fails.length === 0;
  let summary: string;
  if (onlyNudges) {
    summary = `Podium nudged the assistant${times(nudges.length)}`;
  } else {
    const shown = groups.slice(0, SUMMARY_SERVERS).join(", ");
    const more = groups.length - SUMMARY_SERVERS;
    summary = [
      nudges.length > 0 ? "Podium check · " : "",
      tools.length > 0
        ? `Ran ${tools.length} action${tools.length > 1 ? "s" : ""} · ${shown}${more > 0 ? `, +${more}` : ""}`
        : "",
      fails.length > 0
        ? `${tools.length > 0 ? " ⚠ " : "⚠ "}${fails.length}${tools.length > 0 ? "" : " failed"}`
        : "",
    ].join("");
  }

  const nudgeTitle = nudges.map((m) => textPreview(m.text)).join("\n\n");

  // Only nudges: nothing to open, so no disclosure — the text rides on hover.
  if (onlyNudges) {
    return (
      <div className={styles.activity} data-podium="true" title={nudgeTitle}>
        {summary}
      </div>
    );
  }

  return (
    <details
      className={styles.activity}
      data-podium={nudges.length > 0 ? "true" : undefined}
      onToggle={onToggle}
    >
      <summary>{summary}</summary>
      <ul>
        {[...counts].map(([label, n]) => (
          <li key={label}>
            {label}
            {times(n)}
          </li>
        ))}
        {fails.map((m) => (
          <li
            key={m.id}
            className={styles.activityFailed}
            title={textPreview(m.text, 2000)}
          >
            ⚠ action failed: {textPreview(m.text)}
          </li>
        ))}
        {nudges.length > 0 && (
          <li title={nudgeTitle}>
            Podium nudged the assistant{times(nudges.length)}
          </li>
        )}
      </ul>
    </details>
  );
}

/**
 * The user's reply to a Responses item: which item it answers (clipped to
 * one line), then the reply itself, wrapped in full. `text` is
 * `"<label>\n<reply>"`, the shape the backend stores too.
 */
function ReplyLine({ text }: { text: string }) {
  const newline = text.indexOf("\n");
  const label = newline < 0 ? "" : text.slice(0, newline);
  const reply = text.slice(newline + 1);
  return (
    <span className={styles.reply}>
      <ReplyIcon className={styles.replyIcon} />
      <span className={styles.replyLabel}>Re: {label}</span>
      <span className={styles.replyText}>{reply}</span>
    </span>
  );
}

/**
 * One message waiting for the current turn to end. Only a user message is
 * editable/removable — a nudge or proposal-outcome notice is Podium's own
 * text, shown as-is until it is delivered.
 */
function PendingRow({ item }: { item: QueuedMessage }) {
  const editPending = useAssistantStore((s) => s.editPending);
  const sendPendingNow = useAssistantStore((s) => s.sendPendingNow);
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
        <span className={styles.pendingText}>
          {item.replyTo && (
            <>
              <span className={styles.replyLabel}>Re: {item.replyTo}</span>{" "}
            </>
          )}
          {item.text}
        </span>
      )}
      {item.attachments.length > 0 && (
        <span className={styles.pendingAttachments}>
          {item.attachments.map((a) => a.name).join(", ")}
        </span>
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
            aria-label="Send waiting message now"
            title="Send now — it may land mid-answer"
            onClick={() => void sendPendingNow(item.id)}
          >
            <SendIcon />
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
