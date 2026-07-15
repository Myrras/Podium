import type { Editor } from "@tiptap/react";
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { SCRATCHPAD_PLACEHOLDER, ScratchpadEditor } from "./ScratchpadEditor";

/**
 * Drives the real Tiptap/ProseMirror editor in jsdom. jsdom has no layout
 * engine, so `src/test/setup.ts` shims `Range.prototype.getClientRects`/
 * `getBoundingClientRect` — without it, `focus()`/`scrollIntoView()` throw
 * from inside prosemirror-view's `coordsAtPos`.
 */

function waitForEditor(onEditorReady: ReturnType<typeof vi.fn>) {
  return waitFor(() => {
    const editor = latestEditor(onEditorReady);
    expect(editor).not.toBeNull();
    return editor as Editor;
  });
}

function latestEditor(onEditorReady: ReturnType<typeof vi.fn>): Editor | null {
  const calls = onEditorReady.mock.calls;
  for (let i = calls.length - 1; i >= 0; i--) {
    const [editor] = calls[i] as [Editor | null];
    if (editor) return editor;
  }
  return null;
}

describe("ScratchpadEditor", () => {
  it("renders the placeholder when content is empty", async () => {
    const onEditorReady = vi.fn();
    const { container } = render(
      <ScratchpadEditor
        content=""
        onChange={vi.fn()}
        onEditorReady={onEditorReady}
      />,
    );

    await waitForEditor(onEditorReady);

    const empty = container.querySelector("[data-placeholder]");
    expect(empty).not.toBeNull();
    expect(empty?.getAttribute("data-placeholder")).toBe(
      SCRATCHPAD_PLACEHOLDER,
    );
  });

  it("initializes from a markdown content prop", async () => {
    const onEditorReady = vi.fn();
    render(
      <ScratchpadEditor
        content="# Heading\n\nSome **bold** text"
        onChange={vi.fn()}
        onEditorReady={onEditorReady}
      />,
    );

    const editor = await waitForEditor(onEditorReady);
    expect(editor.getText()).toContain("Some bold text");
  });

  it("emits markdown on change", async () => {
    const onChange = vi.fn();
    const onEditorReady = vi.fn();
    render(
      <ScratchpadEditor
        content=""
        onChange={onChange}
        onEditorReady={onEditorReady}
      />,
    );
    const editor = await waitForEditor(onEditorReady);

    act(() => {
      editor.chain().focus().insertContent("Hello world").run();
    });

    await waitFor(() => {
      expect(onChange).toHaveBeenCalledWith(
        expect.stringContaining("Hello world"),
      );
    });
  });

  it("adopts an external content change without emitting an update", async () => {
    const onChange = vi.fn();
    const onEditorReady = vi.fn();
    const { rerender } = render(
      <ScratchpadEditor
        content="Original"
        onChange={onChange}
        onEditorReady={onEditorReady}
      />,
    );
    const editor = await waitForEditor(onEditorReady);
    expect(editor.getText()).toBe("Original");

    rerender(
      <ScratchpadEditor
        content="Updated externally"
        onChange={onChange}
        onEditorReady={onEditorReady}
      />,
    );

    await waitFor(() => {
      expect(editor.getText()).toBe("Updated externally");
    });
    expect(onChange).not.toHaveBeenCalled();
  });

  it("does not reset content or emit when re-rendered with equivalent markdown", async () => {
    const onChange = vi.fn();
    const onEditorReady = vi.fn();
    const { rerender } = render(
      <ScratchpadEditor
        content="Same content"
        onChange={onChange}
        onEditorReady={onEditorReady}
      />,
    );
    const editor = await waitForEditor(onEditorReady);
    const setContentSpy = vi.spyOn(editor.commands, "setContent");

    rerender(
      <ScratchpadEditor
        content="Same content"
        onChange={onChange}
        onEditorReady={onEditorReady}
      />,
    );

    expect(setContentSpy).not.toHaveBeenCalled();
    expect(onChange).not.toHaveBeenCalled();
  });

  describe("markdown round-trip", () => {
    async function roundTrip(markdown: string) {
      const onEditorReady = vi.fn();
      render(
        <ScratchpadEditor
          content={markdown}
          onChange={vi.fn()}
          onEditorReady={onEditorReady}
        />,
      );
      const editor = await waitForEditor(onEditorReady);
      return editor.storage.markdown.getMarkdown();
    }

    it("round-trips bold, italic, and strikethrough", async () => {
      const out = await roundTrip("**bold** _italic_ ~~struck~~");
      expect(out).toContain("**bold**");
      expect(out).toMatch(/[_*]italic[_*]/);
      expect(out).toContain("~~struck~~");
    });

    it("round-trips headings, including H1", async () => {
      const out = await roundTrip("# H1\n\n## H2\n\n### H3");
      expect(out).toContain("# H1");
      expect(out).toContain("## H2");
      expect(out).toContain("### H3");
    });

    it("round-trips bullet and numbered lists", async () => {
      const out = await roundTrip("- one\n- two\n\n1. first\n2. second");
      expect(out).toMatch(/[-*] one/);
      expect(out).toMatch(/[-*] two/);
      expect(out).toContain("1. first");
      expect(out).toContain("2. second");
    });

    it("round-trips a checklist", async () => {
      const out = await roundTrip("- [ ] todo\n- [x] done");
      expect(out).toMatch(/\[ ] todo/);
      expect(out).toMatch(/\[x] done/i);
    });

    it("round-trips a blockquote", async () => {
      const out = await roundTrip("> quoted text");
      expect(out).toContain("> quoted text");
    });

    it("round-trips a code block", async () => {
      const out = await roundTrip("```\nconst x = 1;\n```");
      expect(out).toContain("```");
      expect(out).toContain("const x = 1;");
    });

    it("round-trips a horizontal rule", async () => {
      const out = await roundTrip("above\n\n---\n\nbelow");
      expect(out).toContain("---");
    });

    it("round-trips a link", async () => {
      const out = await roundTrip("[Podium](https://example.com)");
      expect(out).toContain("[Podium](https://example.com)");
    });

    it("round-trips a table", async () => {
      const table =
        "| A | B |\n| --- | --- |\n| one | two |\n| three | four |";
      const out = await roundTrip(table);
      expect(out).toContain("| A | B |");
      expect(out).toContain("| one | two |");
      expect(out).toContain("| three | four |");
    });
  });

  describe("pasting", () => {
    it("parses a pasted heading as a real heading, not literal '##' text", async () => {
      const onEditorReady = vi.fn();
      render(
        <ScratchpadEditor
          content=""
          onChange={vi.fn()}
          onEditorReady={onEditorReady}
        />,
      );
      const editor = await waitForEditor(onEditorReady);

      act(() => {
        // `view.pasteText` forces prosemirror's "paste as plain text" path
        // (`preferPlain: true`), which is exactly the mode the markdown
        // extension's `clipboardTextParser` skips (a user who explicitly
        // asks to paste as plain text wants the literal text). A normal
        // Cmd+V of plain-text-only clipboard data goes through
        // prosemirror-view's real `paste` DOM handler instead, which only
        // sets `preferPlain` from the shift-key state — so dispatch a real
        // `paste` event with a `clipboardData`-like object exposing only
        // `text/plain`, no `text/html`, to exercise that actual path.
        const event = new Event("paste", { bubbles: true, cancelable: true });
        Object.defineProperty(event, "clipboardData", {
          value: {
            getData: (type: string) =>
              type === "text/plain" ? "## Section 3\n\nSome body text" : "",
          },
        });
        editor.view.dom.dispatchEvent(event);
      });

      const html = editor.getHTML();
      expect(html).toContain("<h2>Section 3</h2>");
      expect(html).not.toContain("## Section 3");
    });
  });

  describe("copying", () => {
    it("puts markdown, not plain text, on the clipboard", async () => {
      const onEditorReady = vi.fn();
      render(
        <ScratchpadEditor
          content="# Title\n\n**bold** text"
          onChange={vi.fn()}
          onEditorReady={onEditorReady}
        />,
      );
      const editor = await waitForEditor(onEditorReady);

      const { view } = editor;
      const slice = view.state.doc.slice(0, view.state.doc.content.size);
      const { text } = view.serializeForClipboard(slice);

      expect(text).toContain("# Title");
      expect(text).toContain("**bold**");
    });
  });
});
