/**
 * The changes panel: a right-side island, like the assistant, listing the
 * files changed in one checkout with their diffs inline, for a quick review
 * of what an agent (or you) changed.
 *
 * The checkout follows what is focused — the active process's worktree, or
 * the root of the project whose process/to-do/scratchpad/board is open —
 * until the user picks one from the dropdown, which pins it. The base
 * toggle switches between "Uncommitted" (working tree vs `HEAD`) and
 * "Branch" (everything since the merge-base with the default branch).
 */

import { useEffect, useMemo } from "react";

import type { ChangeStatus, ChangedFile } from "../ipc/types";
import { parseUnifiedDiff } from "../lib/diff";
import type { ChangesTarget, DiffEntry } from "../state/changesStore";
import { targetKey, useChangesStore } from "../state/changesStore";
import { useLayoutStore } from "../state/layoutStore";
import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";
import { useWorktreeStore } from "../state/worktreeStore";
import { CaretIcon, ChangesIcon, CloseIcon, RestartIcon } from "./icons";
import styles from "./ChangesPane.module.css";

/** How often an open panel re-reads git, besides focus and manual refresh. */
const POLL_MS = 10_000;

/** Above this many files "expand all" is not offered (one git call each). */
const MAX_EXPAND_ALL = 50;

const FOLLOW = "__follow";

const STATUS_LETTER: Record<ChangeStatus, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  copied: "C",
  typeChanged: "T",
  untracked: "U",
};

/** The checkout matching what is focused in the work area. */
function useFocusedTarget(): ChangesTarget | null {
  const projects = useProjectStore((s) => s.projects);
  const activeProcess = useProcessStore(
    (s) => s.processes.find((p) => p.id === s.activeProcessId) ?? null,
  );
  const openTodo = useLayoutStore((s) => s.openTodo);
  const openScratchpad = useLayoutStore((s) => s.openScratchpad);
  const openBoard = useLayoutStore((s) => s.openBoard);
  return useMemo(() => {
    const exists = (id: string) => projects.some((p) => p.id === id);
    if (activeProcess && exists(activeProcess.projectId)) {
      return {
        projectId: activeProcess.projectId,
        worktree: activeProcess.worktree,
      };
    }
    const projectId =
      openTodo?.projectId ?? openScratchpad?.projectId ?? openBoard?.projectId;
    if (projectId && exists(projectId)) return { projectId, worktree: null };
    const first = projects[0];
    return first ? { projectId: first.id, worktree: null } : null;
  }, [projects, activeProcess, openTodo, openScratchpad, openBoard]);
}

export function ChangesPane() {
  const width = useLayoutStore((s) => s.changesWidth);
  const setChangesOpen = useLayoutStore((s) => s.setChangesOpen);
  const projects = useProjectStore((s) => s.projects);
  const worktreesByProject = useWorktreeStore((s) => s.worktreesByProject);

  const pinned = useChangesStore((s) => s.pinned);
  const base = useChangesStore((s) => s.base);
  const changes = useChangesStore((s) => s.changes);
  const loading = useChangesStore((s) => s.loading);
  const error = useChangesStore((s) => s.error);
  const expanded = useChangesStore((s) => s.expanded);
  const diffs = useChangesStore((s) => s.diffs);
  const { pin, setBase, load, toggleFile, setAllExpanded } =
    useChangesStore.getState();

  const focused = useFocusedTarget();
  // A pin on a closed project (or a removed worktree) falls back to focus.
  const pinnedValid =
    pinned &&
    projects.some((p) => p.id === pinned.projectId) &&
    (pinned.worktree === null ||
      (worktreesByProject[pinned.projectId] ?? []).some(
        (w) => w.name === pinned.worktree,
      ));
  const target = pinnedValid ? pinned : focused;
  const key = target ? targetKey(target) : null;

  useEffect(() => {
    if (!target) return;
    void load(target);
    const poll = window.setInterval(() => void load(target), POLL_MS);
    const onFocus = () => void load(target);
    window.addEventListener("focus", onFocus);
    return () => {
      window.clearInterval(poll);
      window.removeEventListener("focus", onFocus);
    };
    // `target` is rebuilt every render; `key` + `base` say when it changed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, base]);

  const project = target
    ? projects.find((p) => p.id === target.projectId)
    : undefined;
  const files = changes?.files ?? [];
  const totals = files.reduce(
    (acc, f) => ({
      add: acc.add + (f.additions ?? 0),
      del: acc.del + (f.deletions ?? 0),
    }),
    { add: 0, del: 0 },
  );
  const anyExpanded = Object.values(expanded).some(Boolean);

  function onPick(value: string) {
    if (value === FOLLOW) {
      pin(null);
      return;
    }
    const [projectId, worktree] = value.split("|");
    pin({ projectId, worktree: worktree || null });
  }

  return (
    <aside
      className={styles.pane}
      style={{ width, minWidth: width }}
      aria-label="Changes"
    >
      <header className={styles.header}>
        <ChangesIcon className={styles.headerIcon} />
        <span className={styles.title}>Changes</span>
        <button
          type="button"
          className={styles.iconBtn}
          aria-label="Refresh changes"
          title="Refresh"
          disabled={!target}
          onClick={() => target && void load(target)}
        >
          <RestartIcon size={14} />
        </button>
        <button
          type="button"
          className={styles.iconBtn}
          aria-label="Close changes"
          title="Close"
          onClick={() => setChangesOpen(false)}
        >
          <CloseIcon />
        </button>
      </header>

      <div className={styles.controls}>
        <select
          className={styles.target}
          aria-label="Checkout"
          value={
            pinnedValid && pinned
              ? `${pinned.projectId}|${pinned.worktree ?? ""}`
              : FOLLOW
          }
          onChange={(e) => onPick(e.target.value)}
        >
          <option value={FOLLOW}>
            Follow focus
            {focused
              ? ` (${projects.find((p) => p.id === focused.projectId)?.name ?? ""}${
                  focused.worktree ? ` / ${focused.worktree}` : ""
                })`
              : ""}
          </option>
          {projects.map((p) => (
            <optgroup key={p.id} label={p.name}>
              <option value={`${p.id}|`}>{p.name} — project root</option>
              {(worktreesByProject[p.id] ?? []).map((w) => (
                <option key={w.name} value={`${p.id}|${w.name}`}>
                  {w.name} ({w.branch})
                </option>
              ))}
            </optgroup>
          ))}
        </select>
        <div className={styles.segmented} role="group" aria-label="Compare">
          <button
            type="button"
            aria-pressed={base === "uncommitted"}
            title="Working tree against HEAD, untracked files included"
            onClick={() => setBase("uncommitted")}
          >
            Uncommitted
          </button>
          <button
            type="button"
            aria-pressed={base === "branch"}
            title="Everything since this branch left the default branch, uncommitted included"
            onClick={() => setBase("branch")}
          >
            Branch
          </button>
        </div>
      </div>

      {target && changes && (
        <div className={styles.summary}>
          <span className={styles.summaryRef} title={project?.root}>
            {changes.branch ?? "detached HEAD"}
            {target.worktree ? ` · ${target.worktree}` : ""}
          </span>
          <span className={styles.summaryBase}>
            {changes.base === "branch"
              ? changes.baseRef
                ? `vs ${changes.baseRef}${
                    changes.baseCommit ? ` (${changes.baseCommit})` : ""
                  }`
                : "no default branch found — showing uncommitted"
              : changes.baseCommit
                ? `vs HEAD (${changes.baseCommit})`
                : "no commits yet"}
          </span>
        </div>
      )}

      <div className={styles.list}>
        {!target && (
          <p className={styles.empty}>Open a project to see its changes.</p>
        )}
        {target && error && <p className={styles.error}>{error}</p>}
        {target && !error && !changes && loading && (
          <p className={styles.empty}>Loading…</p>
        )}
        {target && changes && files.length === 0 && (
          <p className={styles.empty}>No changes.</p>
        )}
        {files.length > 0 && (
          <div className={styles.listHeader}>
            <span>
              {files.length} file{files.length === 1 ? "" : "s"}
              {changes?.truncated ? "+" : ""}
            </span>
            <span className={styles.add}>+{totals.add}</span>
            <span className={styles.del}>−{totals.del}</span>
            <span className={styles.spacer} />
            {(anyExpanded || files.length <= MAX_EXPAND_ALL) && (
              <button
                type="button"
                className={styles.linkBtn}
                onClick={() => target && setAllExpanded(target, !anyExpanded)}
              >
                {anyExpanded ? "Collapse all" : "Expand all"}
              </button>
            )}
          </div>
        )}
        {target &&
          files.map((f) => (
            <FileRow
              key={f.path}
              file={f}
              open={!!expanded[f.path]}
              entry={diffs[f.path]}
              onToggle={() => toggleFile(target, f)}
            />
          ))}
        {changes?.truncated && (
          <p className={styles.empty}>
            Only the first {files.length} files are listed.
          </p>
        )}
      </div>
    </aside>
  );
}

function FileRow({
  file,
  open,
  entry,
  onToggle,
}: {
  file: ChangedFile;
  open: boolean;
  entry: DiffEntry | undefined;
  onToggle: () => void;
}) {
  const slash = file.path.lastIndexOf("/");
  const dir = slash >= 0 ? file.path.slice(0, slash + 1) : "";
  const name = file.path.slice(slash + 1);
  return (
    <div className={styles.file}>
      <button
        type="button"
        className={styles.fileRow}
        aria-expanded={open}
        title={file.oldPath ? `${file.oldPath} → ${file.path}` : file.path}
        onClick={onToggle}
      >
        <CaretIcon
          size={12}
          className={styles.caret}
          data-open={open || undefined}
        />
        <span className={styles.status} data-status={file.status}>
          {STATUS_LETTER[file.status]}
        </span>
        <span className={styles.path}>
          <span className={styles.dir}>{dir}</span>
          <span className={styles.name}>{name}</span>
        </span>
        {file.binary ? (
          <span className={styles.counts}>binary</span>
        ) : (
          <span className={styles.counts}>
            {file.additions !== null && (
              <span className={styles.add}>+{file.additions}</span>
            )}
            {file.deletions !== null && file.deletions > 0 && (
              <span className={styles.del}>−{file.deletions}</span>
            )}
          </span>
        )}
      </button>
      {open && <DiffView entry={entry} file={file} />}
    </div>
  );
}

function DiffView({
  entry,
  file,
}: {
  entry: DiffEntry | undefined;
  file: ChangedFile;
}) {
  const rows = useMemo(
    () => (entry?.diff ? parseUnifiedDiff(entry.diff.diff) : []),
    [entry?.diff],
  );
  if (!entry || (entry.loading && !entry.diff)) {
    return <p className={styles.diffNote}>Loading diff…</p>;
  }
  if (entry.error) return <p className={styles.error}>{entry.error}</p>;
  const diff = entry.diff;
  if (!diff) return null;
  if (diff.binary) return <p className={styles.diffNote}>Binary file.</p>;
  if (rows.length === 0) {
    return (
      <p className={styles.diffNote}>
        {file.oldPath
          ? "Renamed without content changes."
          : diff.truncated
            ? "Too large to show."
            : "No content changes."}
      </p>
    );
  }
  return (
    <div className={styles.diff}>
      <table className={styles.diffTable}>
        <tbody>
          {rows.map((r, i) =>
            r.kind === "hunk" ? (
              <tr key={i} className={styles.hunk}>
                <td colSpan={3}>{r.text}</td>
              </tr>
            ) : (
              <tr key={i} className={styles.line} data-kind={r.kind}>
                <td className={styles.num}>{r.oldLine ?? ""}</td>
                <td className={styles.num}>{r.newLine ?? ""}</td>
                <td className={styles.code}>
                  <span className={styles.sign}>
                    {r.kind === "add" ? "+" : r.kind === "del" ? "-" : " "}
                  </span>
                  {r.text}
                </td>
              </tr>
            ),
          )}
        </tbody>
      </table>
      {diff.truncated && (
        <p className={styles.diffNote}>Diff cut short — it is too large.</p>
      )}
    </div>
  );
}
