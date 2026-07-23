/**
 * Work-area pane for an opened scratchpad: a header with an inline-editable
 * title, tag chips, an archive button, and a close button, over a formatting
 * toolbar plus fullscreen toggle and a Tiptap WYSIWYG markdown editor
 * (`ScratchpadEditor` — the persisted value is always plain markdown, Tiptap
 * is purely a view-layer concern) with debounced autosave, and a footer
 * showing when it was last touched. Scratchpads are shared with agents over
 * MCP, so the pane reads the live scratchpad from the store by id and
 * reflects `scratchpad:changed` refreshes while open — external edits are
 * only adopted when there is no unsaved local change pending, so an agent's
 * edit never clobbers text the user is mid-typing.
 *
 * Content/title saves carry the scratchpad's last-known `updatedAt`; if a
 * concurrent edit (the user in another window, or an agent) landed first,
 * the save is rejected as a conflict instead of silently overwriting it —
 * an in-pane banner then offers "Reload" (discard the local edit, adopt the
 * latest) or "Force save" (retry with the fresh timestamp). The toolbar and
 * editor stay mounted (and editable) while the banner is up — the adoption
 * effects below are frozen instead, so the user keeps typing against their
 * own draft until they explicitly choose Reload or Force Save.
 */

import type { Editor } from "@tiptap/react";
import { useEffect, useRef, useState } from "react";

import type { ProjectId, ScratchpadId, ScratchpadInfo } from "../ipc/types";
import { formatUpdatedAt } from "../lib/dateFormat";
import {
  type Heading,
  scrollToHeading,
  useHeadings,
} from "../lib/tiptapHeadings";
import { useLayoutStore } from "../state/layoutStore";
import { useScratchpadStore } from "../state/scratchpadStore";
import { CopyIdButton } from "./CopyIdButton";
import {
  ArchiveIcon,
  CloseIcon,
  CollapseIcon,
  ExpandIcon,
  ScratchpadIcon,
} from "./icons";
import styles from "./ScratchpadDetailPane.module.css";
import { ScratchpadEditor } from "./ScratchpadEditor";
import { ScratchpadTOC } from "./ScratchpadTOC";
import { ScratchpadToolbar } from "./ScratchpadToolbar";
import { TagChip } from "./TagChip";

const NO_SCRATCHPADS: ScratchpadInfo[] = [];

/** Autosave debounce: fires this long after the last keystroke. */
const AUTOSAVE_DELAY_MS = 600;

export function ScratchpadDetailPane({
  projectId,
  scratchpadId,
}: {
  projectId: ProjectId;
  scratchpadId: ScratchpadId;
}) {
  const scratchpad = useScratchpadStore((s) =>
    (s.scratchpadsByProject[projectId] ?? NO_SCRATCHPADS).find(
      (sp) => sp.id === scratchpadId,
    ),
  );
  const updateContent = useScratchpadStore((s) => s.updateContent);
  const updateTitle = useScratchpadStore((s) => s.updateTitle);
  const addTag = useScratchpadStore((s) => s.addTag);
  const removeTag = useScratchpadStore((s) => s.removeTag);
  const setScratchpadArchived = useScratchpadStore(
    (s) => s.setScratchpadArchived,
  );
  const refresh = useScratchpadStore((s) => s.refresh);
  const clearOpenScratchpad = useLayoutStore((s) => s.clearOpenScratchpad);

  const [title, setTitle] = useState(scratchpad?.title ?? "");
  const [content, setContent] = useState(scratchpad?.content ?? "");
  // Whether the last save attempt was rejected as a conflict (someone else
  // edited the scratchpad first) — shows the reload/force-save banner.
  const [conflict, setConflict] = useState(false);
  const [editor, setEditor] = useState<Editor | null>(null);
  const [fullscreen, setFullscreen] = useState(false);
  // "On this page" outline of the current document's H2/H3 headings — keyed
  // off the live `editor` instance, so it starts empty until the editor is
  // ready and resets automatically on every remount (App keys this pane by
  // scratchpad id, giving each scratchpad a fresh `Editor`).
  const headings = useHeadings(editor);
  // The last value we know is in sync with the store (either what we last
  // saved, or the last value pulled in from it) — comparing against this
  // (rather than the live state) is what lets an external refresh update the
  // field without a dependency cycle back through the state it's comparing.
  const savedTitleRef = useRef(scratchpad?.title ?? "");
  const savedContentRef = useRef(scratchpad?.content ?? "");
  // The scratchpad's `updatedAt` this pane last knew to be current — echoed
  // back verbatim as `expectedUpdatedAt` on the next save. Only advances
  // while no edit is in flight (see the sync effect below), so a concurrent
  // edit that lands mid-typing is still caught as a conflict rather than
  // silently adopted as the new "expected" base.
  const expectedUpdatedAtRef = useRef(scratchpad?.updatedAt ?? "");
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // The latest content the textarea holds, kept in a ref (not just closed
  // over by the debounce timer) so the unmount flush below can always save
  // the current value instead of a stale one from an earlier render.
  const latestContentRef = useRef(scratchpad?.content ?? "");
  const projectIdRef = useRef(projectId);
  const scratchpadIdRef = useRef(scratchpadId);
  // Whether the scratchpad still exists in the store, tracked in a ref (not
  // just the closed-over `scratchpad` variable) so the unmount flush below
  // sees the latest answer even though its effect only runs once.
  const existsRef = useRef(scratchpad !== undefined);
  projectIdRef.current = projectId;
  scratchpadIdRef.current = scratchpadId;
  existsRef.current = scratchpad !== undefined;

  // Adopt an external title change (e.g. an agent rename) only when the
  // field has no unsaved local edit pending. Frozen while a conflict banner
  // is up: the `refresh()` the conflict handler triggers pulls in the
  // server's title, but adopting it here would silently overwrite what the
  // user is deciding about — "Reload"/"Force save" apply it explicitly.
  useEffect(() => {
    if (scratchpad === undefined || conflict) return;
    if (scratchpad.title === savedTitleRef.current) return;
    setTitle((current) =>
      current === savedTitleRef.current ? scratchpad.title : current,
    );
    savedTitleRef.current = scratchpad.title;
  }, [scratchpad, conflict]);

  // Same for content — an agent's edit is adopted only while the user isn't
  // mid-typing an unsaved change; otherwise it arrives on the next autosave.
  // Also frozen while a conflict banner is up, for the same reason as title.
  useEffect(() => {
    if (scratchpad === undefined || conflict) return;
    if (scratchpad.content === savedContentRef.current) return;
    setContent((current) =>
      current === savedContentRef.current ? scratchpad.content : current,
    );
    savedContentRef.current = scratchpad.content;
  }, [scratchpad, conflict]);

  // Advance the conflict-detection base only while no save is in flight and
  // no conflict banner is up — if a debounced autosave is pending, this
  // pane's local edit hasn't been sent yet, so silently adopting a newer
  // `updatedAt` here would let that pending save clobber whatever produced
  // it. The `conflict` guard matters too: the banner's `refresh()` pulls in
  // the concurrent edit's fresh `updatedAt`, and without this guard the next
  // autosave (triggered by the user simply continuing to type) would quietly
  // succeed with that fresh timestamp — clobbering the conflicting edit
  // without the user ever choosing Reload or Force Save.
  useEffect(() => {
    if (scratchpad === undefined || conflict) return;
    if (saveTimerRef.current) return;
    expectedUpdatedAtRef.current = scratchpad.updatedAt;
  }, [scratchpad, conflict]);

  // The open scratchpad vanished (removed, or archived here or by an agent):
  // close the pane.
  useEffect(() => {
    if (scratchpad === undefined) clearOpenScratchpad();
  }, [scratchpad, clearOpenScratchpad]);

  // Flush a pending debounced autosave on unmount (closing the pane, or
  // switching to another pane/process) instead of just cancelling it — the
  // debounce timer resets on every keystroke, so a user who types
  // continuously and then immediately closes the pane would otherwise lose
  // the entire unsaved edit. Uses refs (not the closed-over `content`/props)
  // so it always saves the latest value even though this effect only runs
  // once. Skipped if the scratchpad was removed out from under us (e.g. the
  // project closed) — saving would just fail with "not found"; a pending
  // edit is lost in that case (known Phase 1 limitation).
  useEffect(() => {
    return () => {
      if (!saveTimerRef.current) return;
      clearTimeout(saveTimerRef.current);
      if (!existsRef.current) return;
      void updateContent(
        projectIdRef.current,
        scratchpadIdRef.current,
        latestContentRef.current,
        expectedUpdatedAtRef.current,
      );
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (scratchpad === undefined) return null;

  const handleContentChange = (value: string) => {
    setContent(value);
    latestContentRef.current = value;
    if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    saveTimerRef.current = setTimeout(() => {
      saveTimerRef.current = null;
      savedContentRef.current = value;
      const expected = expectedUpdatedAtRef.current;
      void updateContent(projectId, scratchpadId, value, expected).then(
        (result) => {
          if (result === null) return;
          if ("conflict" in result) {
            setConflict(true);
            void refresh(projectId);
            return;
          }
          savedContentRef.current = result.content;
          expectedUpdatedAtRef.current = result.updatedAt;
        },
      );
    }, AUTOSAVE_DELAY_MS);
  };

  const commitTitle = () => {
    const trimmed = title.trim();
    if (trimmed === scratchpad.title) return;
    savedTitleRef.current = trimmed;
    const expected = expectedUpdatedAtRef.current;
    void updateTitle(projectId, scratchpadId, trimmed, expected).then(
      (result) => {
        if (result === null) return;
        if ("conflict" in result) {
          setConflict(true);
          void refresh(projectId);
          return;
        }
        savedTitleRef.current = result.title;
        setTitle(result.title);
        expectedUpdatedAtRef.current = result.updatedAt;
      },
    );
  };

  // Discard the local edit and adopt the latest content/title from the
  // store (the conflict handler already triggered a `refresh`).
  const handleSelectHeading = (heading: Heading) => {
    if (!editor) return;
    scrollToHeading(editor, heading.pos);
  };

  const reloadFromServer = () => {
    setContent(scratchpad.content);
    setTitle(scratchpad.title);
    savedContentRef.current = scratchpad.content;
    savedTitleRef.current = scratchpad.title;
    latestContentRef.current = scratchpad.content;
    expectedUpdatedAtRef.current = scratchpad.updatedAt;
    setConflict(false);
  };

  // Retry the save(s) with the scratchpad's current `updatedAt` (refreshed
  // by the conflict handler), overwriting the concurrent edit with this
  // pane's title and/or content. The conflict can originate from either a
  // title save (`commitTitle`) or a content save (`handleContentChange`), so
  // this must not assume it's always content — the title effect stays
  // frozen while `conflict` is true, so `title`/`scratchpad.title` diverging
  // means there's a pending title edit that also needs to survive Force
  // Save, not just get silently overwritten once the banner clears.
  const forceSave = () => {
    const titleDirty = title.trim() !== scratchpad.title;
    const contentDirty = latestContentRef.current !== scratchpad.content;

    void (async () => {
      let expected = scratchpad.updatedAt;

      if (titleDirty) {
        const result = await updateTitle(
          projectId,
          scratchpadId,
          title.trim(),
          expected,
        );
        if (result === null) return;
        if ("conflict" in result) {
          // Someone edited again in the meantime; refresh and let the user retry.
          void refresh(projectId);
          return;
        }
        savedTitleRef.current = result.title;
        setTitle(result.title);
        expected = result.updatedAt;
      }

      if (contentDirty) {
        const result = await updateContent(
          projectId,
          scratchpadId,
          latestContentRef.current,
          expected,
        );
        if (result === null) return;
        if ("conflict" in result) {
          // Someone edited again in the meantime; refresh and let the user retry.
          void refresh(projectId);
          return;
        }
        savedContentRef.current = result.content;
        expected = result.updatedAt;
      }

      expectedUpdatedAtRef.current = expected;
      setConflict(false);
    })();
  };

  return (
    <div className={styles.pane} data-fullscreen={fullscreen || undefined}>
      <header className={styles.header}>
        <ScratchpadIcon className={styles.kindIcon} />
        <input
          className={styles.titleInput}
          value={title}
          aria-label="Scratchpad title"
          onChange={(e) => setTitle(e.target.value)}
          onBlur={commitTitle}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              commitTitle();
            }
          }}
        />
        <button
          type="button"
          className={styles.expandBtn}
          aria-label={fullscreen ? "Exit fullscreen" : "Fullscreen"}
          title={fullscreen ? "Exit fullscreen" : "Fullscreen"}
          onClick={() => setFullscreen((f) => !f)}
        >
          {fullscreen ? <CollapseIcon /> : <ExpandIcon />}
        </button>
        <CopyIdButton id={scratchpadId} className={styles.closeBtn} />
        <button
          type="button"
          className={styles.closeBtn}
          aria-label="Archive scratchpad"
          title="Archive"
          onClick={() =>
            void setScratchpadArchived(projectId, scratchpadId, true)
          }
        >
          <ArchiveIcon />
        </button>
        <button
          type="button"
          className={styles.closeBtn}
          aria-label="Close scratchpad"
          title="Close"
          onClick={clearOpenScratchpad}
        >
          <CloseIcon />
        </button>
      </header>

      <TagChip
        tags={scratchpad.tags}
        onAdd={(tag) => void addTag(projectId, scratchpadId, tag)}
        onRemove={(tag) => void removeTag(projectId, scratchpadId, tag)}
      />

      {conflict && (
        <div className={styles.conflictBanner} role="alert">
          <span>
            This scratchpad was updated elsewhere while you were editing.
          </span>
          <div className={styles.conflictActions}>
            <button type="button" onClick={reloadFromServer}>
              Reload
            </button>
            <button type="button" onClick={forceSave}>
              Force save
            </button>
          </div>
        </div>
      )}

      <ScratchpadToolbar editor={editor} />

      <div className={styles.body}>
        <div className={styles.editorArea}>
          <ScratchpadEditor
            content={content}
            onChange={handleContentChange}
            onEditorReady={setEditor}
          />
        </div>
        <ScratchpadTOC
          headings={headings}
          onSelectHeading={handleSelectHeading}
        />
      </div>

      <footer className={styles.footer}>
        Updated {formatUpdatedAt(scratchpad.updatedAt)} · by{" "}
        {scratchpad.updatedBy} · V{scratchpad.version}
      </footer>
    </div>
  );
}
