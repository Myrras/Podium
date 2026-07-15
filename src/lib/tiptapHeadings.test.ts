import Placeholder from "@tiptap/extension-placeholder";
import { Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { act, renderHook } from "@testing-library/react";
import { Markdown } from "tiptap-markdown";
import { afterEach, describe, expect, it } from "vitest";

import {
  extractHeadings,
  scrollToHeading,
  useHeadings,
  type Heading,
} from "./tiptapHeadings";

/**
 * `extractHeadings`/`scrollToHeading` operate on the plain `@tiptap/core`
 * `Editor`, so these tests drive a headless instance directly rather than
 * mounting `ScratchpadEditor` — no DOM/React needed, matching the extensions
 * `ScratchpadEditor` actually configures (StarterKit + Markdown) is enough to
 * exercise heading nodes.
 */
function makeEditor(content: string): Editor {
  return new Editor({
    extensions: [
      StarterKit,
      Placeholder.configure({ placeholder: "placeholder" }),
      Markdown.configure({ tightLists: true }),
    ],
    content,
  });
}

describe("extractHeadings", () => {
  let editor: Editor | null = null;

  afterEach(() => {
    editor?.destroy();
    editor = null;
  });

  it("returns an empty list for a document with no headings", () => {
    editor = makeEditor("Just a paragraph, no headings here.");
    expect(extractHeadings(editor)).toEqual([]);
  });

  it("extracts H2/H3 headings in document order with level", () => {
    editor = makeEditor(
      "## First\n\nSome text.\n\n### Nested\n\n## Second\n\nMore text.",
    );
    const headings = extractHeadings(editor);
    expect(headings.map(({ level, text }) => ({ level, text }))).toEqual([
      { level: 2, text: "First" },
      { level: 3, text: "Nested" },
      { level: 2, text: "Second" },
    ]);
    // Positions should be strictly increasing (document order).
    for (let i = 1; i < headings.length; i++) {
      expect(headings[i].pos).toBeGreaterThan(headings[i - 1].pos);
    }
  });

  it("includes H1 (the document title) alongside H2/H3", () => {
    editor = makeEditor("# Title\n\n## Section");
    expect(extractHeadings(editor)).toEqual([
      { level: 1, text: "Title", pos: expect.any(Number) },
      { level: 2, text: "Section", pos: expect.any(Number) },
    ]);
  });

  it("updates when a heading is added", () => {
    editor = makeEditor("## First");
    expect(extractHeadings(editor)).toHaveLength(1);

    editor.commands.setContent("## First\n\n## Second");
    expect(extractHeadings(editor).map((h) => h.text)).toEqual([
      "First",
      "Second",
    ]);
  });

  it("updates when a heading is edited", () => {
    editor = makeEditor("## Original");
    editor.commands.setContent("## Renamed");
    expect(extractHeadings(editor).map((h) => h.text)).toEqual(["Renamed"]);
  });

  it("updates when a heading is removed", () => {
    editor = makeEditor("## First\n\n## Second");
    expect(extractHeadings(editor)).toHaveLength(2);

    editor.commands.setContent("## First");
    expect(extractHeadings(editor).map((h) => h.text)).toEqual(["First"]);
  });
});

describe("scrollToHeading", () => {
  let editor: Editor | null = null;

  afterEach(() => {
    editor?.destroy();
    editor = null;
  });

  it("moves the selection to the heading without changing document content", () => {
    editor = makeEditor("## First\n\nSome text.\n\n## Second\n\nMore text.");
    const before: string = editor.storage.markdown.getMarkdown();
    const headings: Heading[] = extractHeadings(editor);
    const second = headings[1];

    scrollToHeading(editor, second.pos);

    expect(editor.state.selection.from).toBe(
      Math.min(second.pos + 1, editor.state.doc.content.size),
    );
    expect(editor.storage.markdown.getMarkdown()).toBe(before);
  });
});

describe("useHeadings", () => {
  let editor: Editor | null = null;

  afterEach(() => {
    editor?.destroy();
    editor = null;
  });

  it("recomputes on a doc-changing transaction, including a suppressed-emitUpdate setContent", () => {
    editor = makeEditor("## First");
    const { result } = renderHook(() => useHeadings(editor));
    expect(result.current.map((h) => h.text)).toEqual(["First"]);

    // `ScratchpadEditor` adopts external/agent edits via
    // `setContent(content, { emitUpdate: false })`, which suppresses
    // Tiptap's `update` event but still dispatches a transaction — this is
    // exactly the case `useHeadings` must not miss.
    act(() => {
      editor?.commands.setContent("## First\n\n## Second", {
        emitUpdate: false,
      });
    });

    expect(result.current.map((h) => h.text)).toEqual(["First", "Second"]);
  });

  it("does not recompute for a selection-only transaction", () => {
    editor = makeEditor("## First\n\n## Second");
    const { result } = renderHook(() => useHeadings(editor));
    const initial = result.current;

    act(() => {
      if (editor) scrollToHeading(editor, initial[1].pos);
    });

    // Same array reference: no state update fired for the selection change.
    expect(result.current).toBe(initial);
  });

  it("returns an empty list once the editor is gone", () => {
    editor = makeEditor("## First");
    const { result, rerender } = renderHook<Heading[], { e: Editor | null }>(
      ({ e }) => useHeadings(e),
      { initialProps: { e: editor as Editor | null } },
    );
    expect(result.current).toHaveLength(1);

    rerender({ e: null });

    expect(result.current).toEqual([]);
  });
});
