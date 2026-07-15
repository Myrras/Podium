/**
 * Live H1/H2/H3 outline of a Tiptap document, for the scratchpad "On this
 * page" panel.
 *
 * We don't use the official `@tiptap/extension-table-of-contents`: it needs
 * heading-id wiring (assigning `data-toc-id` attrs to heading nodes) plus its
 * own `onUpdate` data store, and mutating heading node attrs would have to be
 * checked against `tiptap-markdown`'s serialization (Phase 2's persisted
 * value is plain markdown — an extra node attribute is more moving parts to
 * verify doesn't leak into the round-trip). A plain `doc.descendants()`
 * traversal is a handful of lines, has no interaction with markdown
 * serialization at all, and is all H1/H2/H3 nesting needs.
 */

import type { Editor } from "@tiptap/react";
import { useEffect, useState } from "react";

export interface Heading {
  /**
   * H1 is the document's title, not a section — there's normally only one,
   * so it renders at the same (unindented) level as H2 in the panel rather
   * than getting its own tier; only H3 is indented under it. See
   * `ScratchpadTOC.module.css`, which has no `data-level="1"` rule (falling
   * through to the same unindented default as level 2).
   */
  level: 1 | 2 | 3;
  text: string;
  /** Position of the heading node in `editor.state.doc`, for click-to-scroll. */
  pos: number;
}

/** Extracts H1/H2/H3 headings from the live document, in document order. */
export function extractHeadings(editor: Editor): Heading[] {
  const headings: Heading[] = [];
  editor.state.doc.descendants((node, pos) => {
    if (node.type.name !== "heading") return true;
    const level = node.attrs.level as number;
    if (level === 1 || level === 2 || level === 3) {
      headings.push({ level, text: node.textContent, pos });
    }
    return true;
  });
  return headings;
}

/**
 * Moves the cursor to a heading and scrolls it into view without altering
 * document content. `pos + 1` lands the selection just inside the heading
 * (a reasonable cursor resting place) rather than immediately before it.
 */
export function scrollToHeading(editor: Editor, pos: number): void {
  editor
    .chain()
    .setTextSelection(Math.min(pos + 1, editor.state.doc.content.size))
    .scrollIntoView()
    .run();
}

/**
 * Live-updating H2/H3 outline of `editor`'s document. Recomputed on every
 * transaction that changes the document — including `setContent(...,
 * {emitUpdate: false})` adoption of an external/agent edit, which suppresses
 * Tiptap's `update` event but still dispatches a transaction, and excluding
 * selection-only transactions (e.g. our own scroll-to-heading, or plain
 * cursor movement) so clicking a TOC entry doesn't recompute the list.
 */
export function useHeadings(editor: Editor | null): Heading[] {
  const [headings, setHeadings] = useState<Heading[]>(() =>
    editor ? extractHeadings(editor) : [],
  );

  useEffect(() => {
    if (!editor) {
      setHeadings([]);
      return;
    }
    const sync = ({
      transaction,
    }: {
      transaction: { docChanged: boolean };
    }) => {
      if (!transaction.docChanged) return;
      setHeadings(extractHeadings(editor));
    };
    setHeadings(extractHeadings(editor));
    editor.on("transaction", sync);
    return () => {
      editor.off("transaction", sync);
    };
  }, [editor]);

  return headings;
}
