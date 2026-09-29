/**
 * The Git board: a Kanban of the project's repo in the work area,
 * opened from the project header's Git button. The project's own columns
 * (GitLab board, GitHub Project) or four defaults (open / in progress / in
 * review / done in the last 14 days), a Mine/All scope toggle, and all-time
 * PR/MR counts. A refresh never blanks it (the last board stays, the refresh
 * icon spins); each failure kind gets its own inline message, no toasts. A
 * card click opens the detail drawer beside the columns. Movable cards drag
 * onto columns that accept their kind (the drawer has a "Move to" select for
 * the keyboard). PRs/MRs linked to an issue show inside its card, and open
 * PRs/MRs carry an approval badge (accented when you approved). Cmd/Ctrl+F
 * focuses the search field, which filters every column; Escape clears it.
 */

import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent,
} from "react";

import { issuesProjectSetupSet, toIpcError } from "../ipc/commands";
import type {
  BoardColumn,
  BoardItem,
  IssueProvider,
  LinkedPr,
  ProjectId,
} from "../ipc/types";
import {
  approvalBadge,
  boardErrorText,
  itemRef,
  matchesSearch,
  prRef,
} from "../lib/issueActions";
import { dragAutoScroll, scrollEdges } from "../lib/scrollEdges";
import {
  moveTargets,
  useIssueStore,
  type ProjectBoard,
} from "../state/issueStore";
import { useLayoutStore } from "../state/layoutStore";
import { useProjectStore } from "../state/projectStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { openExternalLink } from "../lib/links";
import { toastError } from "../state/toastStore";
import styles from "./GitBoard.module.css";
import {
  BoardIcon,
  CheckIcon,
  CloseIcon,
  IssueIcon,
  PrIcon,
  RestartIcon,
  SettingsIcon,
} from "./icons";
import { IssueActions } from "./IssueActions";
import { IssueDetailPane } from "./IssueDetailPane";

/** Stable empty state so the selector doesn't re-render on every store set. */
const NONE: ProjectBoard = {
  board: null,
  error: null,
  loading: false,
  fetchedAt: 0,
  mine: true,
};

/** Shown until the first board arrives; mirrors the backend's defaults. */
const DEFAULT_COLUMNS: BoardColumn[] = [
  { id: "open", title: "Open", color: null, accepts: [] },
  { id: "inProgress", title: "In progress", color: null, accepts: [] },
  { id: "review", title: "In review", color: null, accepts: [] },
  { id: "done", title: "Done (14d)", color: null, accepts: [] },
];

/** A hex colour as is; a GitHub colour name maps to the `--board-<name>` token. */
const cssColor = (c: string) => (c.startsWith("#") ? c : `var(--board-${c})`);

/** Errors with nothing to retry: the remote or CLI setup has to change first. */
const SETUP_ERRORS = new Set([
  "issuesNoRemote",
  "issuesGhUnavailable",
  "issuesUnsupportedHost",
  "issuesHostMismatch",
  "issuesGitlabNotConfigured",
  "issuesGitlabAuth",
]);

/** Failures the user fixes in Settings → Issues. */
const SETTINGS_ERRORS = new Set([
  "issuesGhUnavailable",
  "issuesUnsupportedHost",
  "issuesHostMismatch",
  "issuesGitlabNotConfigured",
  "issuesGitlabAuth",
]);

const openIssueSettings = () =>
  useSettingsRequestStore.getState().open("issues");

const itemKey = (i: BoardItem) => `${i.kind}-${i.number}`;

const fmt = (n: number | null) => (n === null ? "?" : n.toLocaleString());

export function GitBoard({ projectId }: { projectId: ProjectId }) {
  const { board, error, loading, fetchedAt, mine } = useIssueStore(
    (s) => s.byProject[projectId] ?? NONE,
  );
  const refresh = useIssueStore((s) => s.refresh);
  const setMine = useIssueStore((s) => s.setMine);
  const overview = useIssueStore((s) => s.overview);
  const root = useProjectStore(
    (s) => s.projects.find((p) => p.id === projectId)?.root ?? "",
  );
  const clearOpenBoard = useLayoutStore((s) => s.clearOpenBoard);
  const move = useIssueStore((s) => s.move);
  const [selected, setSelected] = useState<string | null>(null);
  const [dragging, setDragging] = useState<BoardItem | null>(null);
  const [query, setQuery] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);

  // The webview has no find bar of its own, so Cmd/Ctrl+F is the board's.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const setups = overview?.setups ?? [];
  const picked = overview?.projects[root] ?? "";
  const pick = (setupId: string) =>
    issuesProjectSetupSet(projectId, setupId || undefined)
      .then((o) => useIssueStore.getState().setOverview(o))
      .catch((e) => toastError(toIpcError(e).message));

  // A setup error means the board (if any) is meaningless; a transient one
  // keeps the stale cards under the message.
  const setupError = error !== null && SETUP_ERRORS.has(error.kind);
  const items = setupError ? [] : (board?.items ?? []);
  const provider = board?.provider ?? "github";
  const prWord = provider === "gitlab" ? "MRs" : "PRs";
  const ref = (i: BoardItem) => itemRef(i, provider);
  const shown = items.filter((i) => matchesSearch(i, ref(i), query));
  const firstLoad = loading && fetchedAt === 0;
  const open = selected ? items.find((i) => itemKey(i) === selected) : null;
  const openRef = selected
    ? ((open && ref(open)) ?? `#${selected.split("-")[1]}`)
    : "";

  return (
    <div className={styles.board}>
      <div className={styles.header}>
        <BoardIcon className={styles.kindIcon} />
        <span className={styles.title}>Git</span>
        <div className={styles.scope} role="group" aria-label="Scope">
          <button
            type="button"
            aria-pressed={mine}
            onClick={() => !mine && setMine(projectId, true)}
          >
            Mine
          </button>
          <button
            type="button"
            aria-pressed={!mine}
            onClick={() => mine && setMine(projectId, false)}
          >
            All
          </button>
        </div>
        {board && !setupError && (
          <span className={styles.counts} aria-label={`${prWord} counts`}>
            {prWord}: <strong>{fmt(board.prCounts.open)}</strong> open ·{" "}
            <strong>{fmt(board.prCounts.merged)}</strong> merged
          </span>
        )}
        {board?.source && !setupError && (
          <span className={styles.source}>{board.source}</span>
        )}
        <span className={styles.spacer} />
        <input
          ref={searchRef}
          type="search"
          className={styles.search}
          placeholder="Search (⌘F)"
          aria-label="Search board"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              setQuery("");
              e.currentTarget.blur();
            }
          }}
        />
        {setups.length > 0 && (
          <select
            className={styles.picker}
            aria-label="Issue setup"
            value={picked}
            onChange={(e) => void pick(e.target.value)}
          >
            <option value="">Auto (match remote)</option>
            {setups.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
              </option>
            ))}
          </select>
        )}
        <button
          type="button"
          className={styles.action}
          aria-label="Refresh board"
          title="Refresh"
          aria-busy={loading}
          onClick={() => void refresh(projectId)}
        >
          <RestartIcon
            size={14}
            className={loading ? styles.refreshing : undefined}
          />
        </button>
        <button
          type="button"
          className={styles.action}
          aria-label="Issue settings"
          title="Issue settings"
          onClick={openIssueSettings}
        >
          <SettingsIcon size={14} />
        </button>
        <button
          type="button"
          className={styles.action}
          aria-label="Close board"
          title="Close"
          onClick={clearOpenBoard}
        >
          <CloseIcon size={14} />
        </button>
      </div>
      {board?.hint && !setupError && (
        <div className={styles.hint} role="note">
          {board.hint}
        </div>
      )}
      {error && (
        <div className={styles.error} role="status">
          <span>{boardErrorText(error)}</span>
          {SETTINGS_ERRORS.has(error.kind) && (
            <button type="button" onClick={openIssueSettings}>
              Check Settings
            </button>
          )}
          {!setupError && (
            <button type="button" onClick={() => void refresh(projectId)}>
              Retry
            </button>
          )}
        </div>
      )}
      {!error && !firstLoad && board && items.length === 0 && (
        <div className={styles.error} role="status">
          {mine
            ? "No open issues or PRs of yours."
            : "No open issues or PRs in this repo."}
        </div>
      )}
      <div className={styles.body}>
        {!setupError && (
          <div
            ref={scrollEdges}
            className={styles.columns}
            onDragOver={(e) => dragging && dragAutoScroll(e, "x")}
          >
            {(board?.columns ?? DEFAULT_COLUMNS).map((col) => {
              const cards = shown.filter((i) => i.column === col.id);
              const valid =
                dragging !== null && moveTargets(dragging, [col]).length > 0;
              const drop = (e: DragEvent) => {
                e.preventDefault();
                const item = dragging;
                setDragging(null);
                if (item && valid) void move(projectId, item, col.id);
              };
              return (
                <section
                  key={col.id}
                  className={styles.column}
                  aria-label={col.title}
                  data-column={col.id}
                  data-drop={dragging ? String(valid) : undefined}
                  onDragOver={(e) => valid && e.preventDefault()}
                  onDrop={drop}
                  style={
                    col.color
                      ? ({
                          "--column-color": cssColor(col.color),
                        } as CSSProperties)
                      : undefined
                  }
                >
                  <h3 className={styles.columnTitle}>
                    <span className={styles.columnDot} aria-hidden />
                    {col.title}
                    <span className={styles.columnCount}>{cards.length}</span>
                  </h3>
                  <div
                    ref={scrollEdges}
                    className={styles.cards}
                    onDragOver={(e) => dragging && dragAutoScroll(e, "y")}
                  >
                    {firstLoad && (
                      <div className={styles.placeholder}>Loading…</div>
                    )}
                    {cards.map((item) => (
                      <Card
                        key={itemKey(item)}
                        projectId={projectId}
                        item={item}
                        itemRef={ref(item)}
                        provider={provider}
                        onDragState={setDragging}
                        showAuthor={!mine}
                        me={board?.me ?? null}
                        selected={selected === itemKey(item)}
                        onOpen={() => setSelected(itemKey(item))}
                      />
                    ))}
                  </div>
                </section>
              );
            })}
          </div>
        )}
        {selected && (
          <IssueDetailPane
            projectId={projectId}
            item={open ?? undefined}
            itemRef={openRef}
            onClose={() => setSelected(null)}
          />
        )}
      </div>
    </div>
  );
}

function Card({
  projectId,
  item,
  itemRef,
  provider,
  onDragState,
  showAuthor,
  me,
  selected,
  onOpen,
}: {
  projectId: ProjectId;
  item: BoardItem;
  itemRef: string;
  provider: IssueProvider;
  onDragState: (item: BoardItem | null) => void;
  showAuthor: boolean;
  me: string | null;
  selected: boolean;
  onOpen: () => void;
}) {
  const Icon = item.kind === "pr" ? PrIcon : IssueIcon;
  const approval = item.column !== "done" && approvalBadge(item.approvals, me);
  return (
    <div
      className={styles.card}
      data-selected={selected ? "true" : undefined}
      draggable={item.movable}
      onDragStart={(e) => {
        e.dataTransfer.effectAllowed = "move";
        e.dataTransfer.setData("text/plain", itemRef);
        onDragState(item);
      }}
      onDragEnd={() => onDragState(null)}
    >
      <button
        type="button"
        className={styles.cardMain}
        onClick={onOpen}
        title={item.title}
      >
        <span className={styles.cardRef}>
          <Icon size={12} />
          {itemRef}
          {item.draft && <span className={styles.draft}>draft</span>}
          {approval && <Approval {...approval} />}
        </span>
        <span className={styles.cardTitle}>{item.title}</span>
        {(item.labels.length > 0 || (showAuthor && item.author)) && (
          <span className={styles.cardMeta}>
            {item.labels.map((l) => (
              <span key={l} className={styles.label}>
                {l}
              </span>
            ))}
            {showAuthor && item.author && (
              <span className={styles.author}>{item.author}</span>
            )}
          </span>
        )}
      </button>
      {item.linked.length > 0 && (
        <ul className={styles.linked} aria-label="Linked">
          {item.linked.map((pr) => (
            <li key={pr.number}>
              <button
                type="button"
                className={styles.linkedPr}
                title={pr.title}
                aria-label={`Open ${prRef(pr.number, provider)} in browser`}
                draggable={false}
                onClick={() => openExternalLink(pr.url)}
              >
                <PrIcon size={11} />
                {prRef(pr.number, provider)}
                {pr.merged ? (
                  <span className={styles.draft}>merged</span>
                ) : (
                  pr.draft && <span className={styles.draft}>draft</span>
                )}
                {!pr.merged && <LinkedApproval pr={pr} me={me} />}
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className={styles.cardActions}>
        <IssueActions
          projectId={projectId}
          issue={item}
          itemRef={itemRef}
          className={styles.action}
        />
      </div>
    </div>
  );
}

function LinkedApproval({ pr, me }: { pr: LinkedPr; me: string | null }) {
  const approval = approvalBadge(pr.approvals, me);
  return approval && <Approval {...approval} />;
}

/** `✓ 1/2`; accented when you are one of the approvers. */
function Approval({
  text,
  mine,
  title,
}: {
  text: string;
  mine: boolean;
  title: string;
}) {
  return (
    <span
      className={styles.approval}
      data-mine={mine ? "true" : undefined}
      title={title}
      aria-label={`Approvals ${text}${mine ? ", including yours" : ""}`}
    >
      <CheckIcon size={11} />
      {text}
    </span>
  );
}
