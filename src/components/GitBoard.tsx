/**
 * The Git board: a read-only Kanban of the project's repo in the work area,
 * opened from the sidebar's Git tab. Four columns (open / in progress / in
 * review / done in the last 14 days), a Mine/All scope toggle, and all-time
 * PR/MR counts. A refresh never blanks it (the last board stays, the refresh
 * icon spins); each failure kind gets its own inline message, no toasts. A
 * card click opens the detail drawer beside the columns.
 */

import { useState } from "react";

import { issuesProjectSetupSet, toIpcError } from "../ipc/commands";
import type { BoardColumn, BoardItem, ProjectId } from "../ipc/types";
import { boardErrorText, itemRef } from "../lib/issueActions";
import { useIssueStore, type ProjectBoard } from "../state/issueStore";
import { useLayoutStore } from "../state/layoutStore";
import { useProjectStore } from "../state/projectStore";
import { useSettingsRequestStore } from "../state/settingsRequestStore";
import { toastError } from "../state/toastStore";
import styles from "./GitBoard.module.css";
import {
  BoardIcon,
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

const COLUMNS: { key: BoardColumn; title: string }[] = [
  { key: "open", title: "Open" },
  { key: "inProgress", title: "In progress" },
  { key: "review", title: "In review" },
  { key: "done", title: "Done (14d)" },
];

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
  const [selected, setSelected] = useState<string | null>(null);

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
        <span className={styles.spacer} />
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
          <div className={styles.columns}>
            {COLUMNS.map((col) => {
              const cards = items.filter((i) => i.column === col.key);
              return (
                <section
                  key={col.key}
                  className={styles.column}
                  aria-label={col.title}
                >
                  <h3 className={styles.columnTitle}>
                    {col.title}
                    <span className={styles.columnCount}>{cards.length}</span>
                  </h3>
                  <div className={styles.cards}>
                    {firstLoad && (
                      <div className={styles.placeholder}>Loading…</div>
                    )}
                    {cards.map((item) => (
                      <Card
                        key={itemKey(item)}
                        projectId={projectId}
                        item={item}
                        itemRef={ref(item)}
                        showAuthor={!mine}
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
  showAuthor,
  selected,
  onOpen,
}: {
  projectId: ProjectId;
  item: BoardItem;
  itemRef: string;
  showAuthor: boolean;
  selected: boolean;
  onOpen: () => void;
}) {
  const Icon = item.kind === "pr" ? PrIcon : IssueIcon;
  return (
    <div className={styles.card} data-selected={selected ? "true" : undefined}>
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
