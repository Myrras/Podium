/**
 * "Scratchpads" subsection for one sidebar project group — freeform notes
 * shared with agents over MCP (title only here; content editing happens in
 * the detail pane). The header "+" immediately creates a new scratchpad
 * (auto-titled) and opens it in the detail pane, since there is no text to
 * capture up front (unlike a to-do). Clicking a row's title opens that
 * scratchpad; hover-revealed spawn-agent and archive buttons sit alongside
 * (see the Archive modal, opened from the header).
 *
 * Cmd/Ctrl+click a row's title toggles it into a selection (Shift+click
 * extends a range from the last-clicked row); with 2+ selected, a bar appears
 * to hand them all to one agent. Unlike to-dos, spawning on a scratchpad
 * always opens the agent picker modal (no direct-spawn shortcut) — scratchpad
 * content can be long-lived context worth reviewing before an agent starts on
 * it.
 */

import { useEffect, useMemo, useRef, useState } from "react";

import type { ProjectId, ScratchpadId, ScratchpadInfo } from "../ipc/types";
import { useProjectStore } from "../state/projectStore";
import { useScratchpadStore } from "../state/scratchpadStore";
import {
  AddIcon,
  AgentIcon,
  ArchiveIcon,
  CloseIcon,
  ScratchpadIcon,
} from "./icons";
import { ScratchpadArchiveModal } from "./ScratchpadArchiveModal";
import sidebarStyles from "./Sidebar.module.css";
import styles from "./TodoSubsection.module.css";

/** Stable empty list so the selector doesn't re-render on every store set. */
const NO_SCRATCHPADS: ScratchpadInfo[] = [];

interface ScratchpadRowProps {
  scratchpad: ScratchpadInfo;
  selected: boolean;
  /** Plain click opens; Cmd/Ctrl or Shift click drives selection. */
  onActivate: (e: React.MouseEvent) => void;
  /** Always opens the agent picker for this one scratchpad. */
  onSpawn: () => void;
  onArchive: () => void;
}

/** One scratchpad: a click-to-open title, and hover actions. */
function ScratchpadRow({
  scratchpad,
  selected,
  onActivate,
  onSpawn,
  onArchive,
}: ScratchpadRowProps) {
  return (
    <div className={styles.row} data-selected={selected ? "true" : undefined}>
      <div className={styles.rowMain}>
        <button
          type="button"
          className={styles.textToggle}
          title={`Open "${scratchpad.title}" — Cmd/Ctrl+click to select`}
          onClick={onActivate}
        >
          <span className={styles.text}>{scratchpad.title}</span>
        </button>
        <button
          type="button"
          className={styles.action}
          aria-label={`Start an agent on "${scratchpad.title}"`}
          title="Start an agent on this scratchpad"
          onClick={onSpawn}
        >
          <AgentIcon size={13} />
        </button>
        <button
          type="button"
          className={styles.action}
          aria-label={`Archive scratchpad "${scratchpad.title}"`}
          title="Archive scratchpad"
          onClick={onArchive}
        >
          <ArchiveIcon size={13} />
        </button>
      </div>
    </div>
  );
}

interface ScratchpadSubsectionProps {
  projectId: ProjectId;
  /** Open the scratchpad detail pane (hosted by the app work area). */
  onOpenScratchpad: (projectId: ProjectId, scratchpadId: ScratchpadId) => void;
  /**
   * Open the agent picker (Scratchpad agent modal) pre-filled for these
   * scratchpad(s). Always used for spawning — scratchpads have no
   * direct-spawn shortcut, unlike to-dos.
   */
  onPickAgent: (
    projectId: ProjectId,
    scratchpadIds: ScratchpadId[],
    initialName: string,
  ) => void;
}

export function ScratchpadSubsection({
  projectId,
  onOpenScratchpad,
  onPickAgent,
}: ScratchpadSubsectionProps) {
  const scratchpads = useScratchpadStore(
    (s) => s.scratchpadsByProject[projectId] ?? NO_SCRATCHPADS,
  );
  const refresh = useScratchpadStore((s) => s.refresh);
  const addScratchpad = useScratchpadStore((s) => s.addScratchpad);
  const setScratchpadArchived = useScratchpadStore(
    (s) => s.setScratchpadArchived,
  );
  const setActiveProject = useProjectStore((s) => s.setActiveProject);

  const [archiveOpen, setArchiveOpen] = useState(false);

  // Multi-select state: the chosen ids and the anchor row for Shift+range.
  const [selected, setSelected] = useState<Set<ScratchpadId>>(new Set());
  const anchorRef = useRef<ScratchpadId | null>(null);

  // Initial pull; later changes arrive via the `scratchpad:changed` refresh.
  useEffect(() => {
    void refresh(projectId);
  }, [projectId, refresh]);

  // Drop ids that no longer exist (a scratchpad was removed/archived away),
  // so the selection and its count never reference stale rows.
  useEffect(() => {
    setSelected((prev) => {
      if (prev.size === 0) return prev;
      const live = new Set(scratchpads.map((sp) => sp.id));
      const next = new Set([...prev].filter((id) => live.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [scratchpads]);

  // Ids to spawn on, in list order (stable, matches what the user sees).
  const selectedIds = useMemo(
    () => scratchpads.filter((sp) => selected.has(sp.id)).map((sp) => sp.id),
    [scratchpads, selected],
  );

  const addAndOpen = async () => {
    const info = await addScratchpad(projectId);
    if (info) onOpenScratchpad(projectId, info.id);
  };

  // A row's title click: Cmd/Ctrl toggles selection, Shift extends a range
  // from the anchor, a plain click opens the scratchpad. A plain click is a
  // "view" gesture, not a selection one, so it leaves any existing selection
  // (and its anchor) intact — clearing is done via the selection bar's clear
  // button or by toggling rows off.
  const activateScratchpad = (
    e: React.MouseEvent,
    scratchpadId: ScratchpadId,
  ) => {
    if (e.metaKey || e.ctrlKey) {
      setSelected((prev) => {
        const next = new Set(prev);
        if (next.has(scratchpadId)) next.delete(scratchpadId);
        else next.add(scratchpadId);
        return next;
      });
      anchorRef.current = scratchpadId;
      return;
    }
    if (e.shiftKey && anchorRef.current) {
      const from = scratchpads.findIndex((sp) => sp.id === anchorRef.current);
      const to = scratchpads.findIndex((sp) => sp.id === scratchpadId);
      if (from !== -1 && to !== -1) {
        const [lo, hi] = from <= to ? [from, to] : [to, from];
        setSelected((prev) => {
          const next = new Set(prev);
          for (let i = lo; i <= hi; i++) next.add(scratchpads[i].id);
          return next;
        });
        return;
      }
    }
    onOpenScratchpad(projectId, scratchpadId);
  };

  // Always opens the picker — scratchpads have no direct-spawn shortcut.
  const spawnOnScratchpad = (scratchpad: ScratchpadInfo) => {
    setActiveProject(projectId);
    onPickAgent(projectId, [scratchpad.id], scratchpad.title);
  };

  const spawnOnSelected = () => {
    if (selectedIds.length === 0) return;
    setActiveProject(projectId);
    const first = scratchpads.find((sp) => sp.id === selectedIds[0]);
    onPickAgent(projectId, selectedIds, first?.title ?? "");
    setSelected(new Set());
    anchorRef.current = null;
  };

  return (
    <div className={sidebarStyles.subsection}>
      <div className={sidebarStyles.sectionHeader}>
        <ScratchpadIcon className={sidebarStyles.panelIcon} />
        <span className={sidebarStyles.panelTitle}>Scratchpads</span>
        <button
          type="button"
          className={sidebarStyles.addBtn}
          aria-label="View archived scratchpads"
          title="Archived scratchpads"
          onClick={() => setArchiveOpen(true)}
        >
          <ArchiveIcon size={13} />
        </button>
        <button
          type="button"
          className={sidebarStyles.addBtn}
          aria-label="New scratchpad"
          title="New scratchpad"
          onClick={() => void addAndOpen()}
        >
          <AddIcon size={13} />
        </button>
      </div>
      {scratchpads.length > 0 ? (
        <div className={styles.rows}>
          {scratchpads.map((sp) => (
            <ScratchpadRow
              key={sp.id}
              scratchpad={sp}
              selected={selected.has(sp.id)}
              onActivate={(e) => activateScratchpad(e, sp.id)}
              onSpawn={() => spawnOnScratchpad(sp)}
              onArchive={() =>
                void setScratchpadArchived(projectId, sp.id, true)
              }
            />
          ))}
        </div>
      ) : (
        <div className={sidebarStyles.placeholder}>No scratchpads yet.</div>
      )}
      {selectedIds.length >= 2 && (
        <div className={styles.selectionBar}>
          <button
            type="button"
            className={styles.selectionSpawn}
            title="Start one agent on all selected scratchpads"
            onClick={spawnOnSelected}
          >
            <AgentIcon size={13} />
            Start agent on {selectedIds.length} scratchpads
          </button>
          <button
            type="button"
            className={styles.selectionClear}
            aria-label="Clear selection"
            title="Clear selection"
            onClick={() => {
              setSelected(new Set());
              anchorRef.current = null;
            }}
          >
            <CloseIcon size={12} />
          </button>
        </div>
      )}
      <ScratchpadArchiveModal
        open={archiveOpen}
        projectId={projectId}
        onClose={() => setArchiveOpen(false)}
      />
    </div>
  );
}
