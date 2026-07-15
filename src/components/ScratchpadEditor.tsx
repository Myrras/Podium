/**
 * WYSIWYG markdown editor for scratchpads, built on Tiptap.
 *
 * The persisted/wire value is always a plain markdown string — Tiptap is
 * purely a view-layer concern. We use `tiptap-markdown` (a markdown-it +
 * `prosemirror-markdown` bridge) so the editor's `content` option accepts a
 * markdown string directly and `editor.storage.markdown.getMarkdown()`
 * serializes the current doc back to markdown; StarterKit + explicit task
 * list/item, table, link, and placeholder extensions cover every formatting
 * action in the toolbar (StarterKit alone lacks checklists, tables, links in
 * some versions, and the empty-state placeholder). `tiptap-markdown` has
 * first-class GFM table serialization built in (it's tested against these
 * same `@tiptap/extension-table*` packages upstream), so tables round-trip
 * to/from markdown with no extra configuration beyond registering them.
 *
 * `ScratchpadEditor` owns the `useEditor` instance internally (so callers
 * only ever deal in markdown strings) but hands the live `Editor` instance
 * back via `onEditorReady` so a sibling `ScratchpadToolbar` can drive
 * formatting commands and reflect active state at the cursor.
 */

import Link from "@tiptap/extension-link";
import Placeholder from "@tiptap/extension-placeholder";
import { TableKit } from "@tiptap/extension-table";
import TaskItem from "@tiptap/extension-task-item";
import TaskList from "@tiptap/extension-task-list";
import { type Editor, EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { useEffect } from "react";
import { Markdown } from "tiptap-markdown";

export const SCRATCHPAD_PLACEHOLDER =
  "Click to type. Notes, research, or handoff details. Markdown supported.";

export function ScratchpadEditor({
  content,
  onChange,
  onEditorReady,
}: {
  content: string;
  onChange: (markdown: string) => void;
  onEditorReady?: (editor: Editor | null) => void;
}) {
  const editor = useEditor({
    extensions: [
      StarterKit,
      Link.configure({ openOnClick: false, autolink: true }),
      TaskList,
      TaskItem.configure({ nested: true }),
      TableKit.configure({ table: { resizable: true } }),
      Placeholder.configure({ placeholder: SCRATCHPAD_PLACEHOLDER }),
      Markdown.configure({
        tightLists: true,
        // Without this, pasted text is inserted as a literal paragraph (a
        // pasted "## Heading" shows up as the raw characters "## Heading"
        // instead of becoming a real heading) — paste doesn't go through
        // Tiptap's typing input rules, only `transformPastedText` does.
        transformPastedText: true,
        // Symmetric with the above: without this, copying rich content out
        // of the editor puts plain text on the clipboard (bold/headings/etc.
        // silently stripped) instead of the markdown source — surprising
        // since the persisted value *is* markdown and paste already
        // round-trips it.
        transformCopiedText: true,
      }),
    ],
    content,
    editorProps: {
      attributes: {
        "aria-label": "Scratchpad content",
        class: "scratchpad-editor-content",
      },
    },
    onUpdate: ({ editor: e }) => {
      onChange(e.storage.markdown.getMarkdown());
    },
  });

  // Hand the live instance up to the toolbar. Runs on every render (cheap —
  // just a ref-style callback) so the toolbar always has the current editor
  // even across Tiptap's own internal re-creates.
  useEffect(() => {
    onEditorReady?.(editor ?? null);
    return () => onEditorReady?.(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editor]);

  // Adopt external content changes (e.g. an agent's edit arriving over MCP)
  // without clobbering in-progress local typing or resetting the cursor: only
  // push a `setContent` when the incoming markdown actually differs from
  // what the editor would currently serialize back out. This is deliberately
  // a second reconciliation layer: ScratchpadDetailPane already gates
  // whether an incoming `content` prop update happens at all (skipping it
  // while there's an unsaved local edit pending); this effect's own
  // markdown-equality check is what makes that adoption idempotent once it
  // does happen — e.g. an echo of our own last save shouldn't reset the
  // cursor even though the parent didn't filter it out.
  useEffect(() => {
    if (!editor) return;
    const current: string = editor.storage.markdown.getMarkdown();
    if (current === content) return;
    editor.commands.setContent(content, { emitUpdate: false });
  }, [editor, content]);

  return (
    <EditorContent
      editor={editor}
      className="scratchpad-editor-content-wrapper"
    />
  );
}
