import type { Editor } from "@tiptap/react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ScratchpadToolbar } from "./ScratchpadToolbar";

/**
 * The toolbar is a thin dispatcher over the live Tiptap `Editor`, so it is
 * tested against a mocked editor object (spying on the chained-command
 * calls and stubbing `isActive`) rather than a real ProseMirror instance —
 * round-trip/rendering behaviour of the editor itself lives in
 * `ScratchpadEditor.test.tsx`.
 */

function mockEditor(
  overrides: {
    isActive?: (...args: unknown[]) => boolean;
    getAttributes?: (...args: unknown[]) => Record<string, unknown>;
  } = {},
): { editor: Editor; chain: ReturnType<typeof vi.fn> } {
  const commandSpy: Record<string, ReturnType<typeof vi.fn>> = {};
  const chainable = {
    focus: vi.fn(() => chainable),
    toggleBold: vi.fn(() => chainable),
    toggleItalic: vi.fn(() => chainable),
    toggleStrike: vi.fn(() => chainable),
    toggleCode: vi.fn(() => chainable),
    toggleHeading: vi.fn(() => chainable),
    toggleBulletList: vi.fn(() => chainable),
    toggleOrderedList: vi.fn(() => chainable),
    toggleTaskList: vi.fn(() => chainable),
    toggleBlockquote: vi.fn(() => chainable),
    toggleCodeBlock: vi.fn(() => chainable),
    setHorizontalRule: vi.fn(() => chainable),
    extendMarkRange: vi.fn(() => chainable),
    setLink: vi.fn(() => chainable),
    unsetLink: vi.fn(() => chainable),
    insertTable: vi.fn(() => chainable),
    deleteTable: vi.fn(() => chainable),
    addRowAfter: vi.fn(() => chainable),
    addColumnAfter: vi.fn(() => chainable),
    run: vi.fn(() => true),
  };
  Object.assign(commandSpy, chainable);

  const chain = vi.fn(() => chainable);
  const editor = {
    chain,
    isActive: overrides.isActive ?? vi.fn(() => false),
    getAttributes: overrides.getAttributes ?? vi.fn(() => ({})),
    on: vi.fn(),
    off: vi.fn(),
  } as unknown as Editor;

  return { editor, chain };
}

describe("ScratchpadToolbar", () => {
  it("renders disabled buttons when there is no editor yet", () => {
    render(<ScratchpadToolbar editor={null} />);
    expect(screen.getByLabelText("Bold")).toBeDisabled();
  });

  it("toggles bold", () => {
    const { editor, chain } = mockEditor();
    render(<ScratchpadToolbar editor={editor} />);

    fireEvent.click(screen.getByLabelText("Bold"));

    expect(chain).toHaveBeenCalled();
    const chainable = chain.mock.results[0].value;
    expect(chainable.toggleBold).toHaveBeenCalled();
    expect(chainable.run).toHaveBeenCalled();
  });

  it("toggles italic", () => {
    const { editor, chain } = mockEditor();
    render(<ScratchpadToolbar editor={editor} />);
    fireEvent.click(screen.getByLabelText("Italic"));
    expect(chain.mock.results[0].value.toggleItalic).toHaveBeenCalled();
  });

  it("toggles strikethrough", () => {
    const { editor, chain } = mockEditor();
    render(<ScratchpadToolbar editor={editor} />);
    fireEvent.click(screen.getByLabelText("Strikethrough"));
    expect(chain.mock.results[0].value.toggleStrike).toHaveBeenCalled();
  });

  it("toggles inline code", () => {
    const { editor, chain } = mockEditor();
    render(<ScratchpadToolbar editor={editor} />);
    fireEvent.click(screen.getByLabelText("Inline code"));
    expect(chain.mock.results[0].value.toggleCode).toHaveBeenCalled();
  });

  it("sets a link via prompt", () => {
    const { editor, chain } = mockEditor();
    vi.spyOn(window, "prompt").mockReturnValue("https://example.com");
    render(<ScratchpadToolbar editor={editor} />);

    fireEvent.click(screen.getByLabelText("Link"));

    const chainable = chain.mock.results[0].value;
    expect(chainable.setLink).toHaveBeenCalledWith({
      href: "https://example.com",
    });
  });

  it("unsets a link when the prompt is cleared", () => {
    const { editor, chain } = mockEditor();
    vi.spyOn(window, "prompt").mockReturnValue("");
    render(<ScratchpadToolbar editor={editor} />);

    fireEvent.click(screen.getByLabelText("Link"));

    expect(chain.mock.results[0].value.unsetLink).toHaveBeenCalled();
  });

  it("toggles heading 1, heading 2, and heading 3", () => {
    const { editor, chain } = mockEditor();
    render(<ScratchpadToolbar editor={editor} />);

    fireEvent.click(screen.getByLabelText("Heading 1"));
    expect(chain.mock.results[0].value.toggleHeading).toHaveBeenCalledWith({
      level: 1,
    });

    fireEvent.click(screen.getByLabelText("Heading 2"));
    expect(chain.mock.results[1].value.toggleHeading).toHaveBeenCalledWith({
      level: 2,
    });

    fireEvent.click(screen.getByLabelText("Heading 3"));
    expect(chain.mock.results[2].value.toggleHeading).toHaveBeenCalledWith({
      level: 3,
    });
  });

  it("toggles bullet list, numbered list, and checklist", () => {
    const { editor, chain } = mockEditor();
    render(<ScratchpadToolbar editor={editor} />);

    fireEvent.click(screen.getByLabelText("Bullet list"));
    expect(chain.mock.results[0].value.toggleBulletList).toHaveBeenCalled();

    fireEvent.click(screen.getByLabelText("Numbered list"));
    expect(chain.mock.results[1].value.toggleOrderedList).toHaveBeenCalled();

    fireEvent.click(screen.getByLabelText("Checklist"));
    expect(chain.mock.results[2].value.toggleTaskList).toHaveBeenCalled();
  });

  it("toggles blockquote and code block, and inserts a horizontal rule", () => {
    const { editor, chain } = mockEditor();
    render(<ScratchpadToolbar editor={editor} />);

    fireEvent.click(screen.getByLabelText("Blockquote"));
    expect(chain.mock.results[0].value.toggleBlockquote).toHaveBeenCalled();

    fireEvent.click(screen.getByLabelText("Code block"));
    expect(chain.mock.results[1].value.toggleCodeBlock).toHaveBeenCalled();

    fireEvent.click(screen.getByLabelText("Horizontal rule"));
    expect(chain.mock.results[2].value.setHorizontalRule).toHaveBeenCalled();
  });

  it("inserts a 3x3 table with a header row when not in a table", () => {
    const { editor, chain } = mockEditor();
    render(<ScratchpadToolbar editor={editor} />);

    fireEvent.click(screen.getByLabelText("Table"));

    expect(chain.mock.results[0].value.insertTable).toHaveBeenCalledWith({
      rows: 3,
      cols: 3,
      withHeaderRow: true,
    });
    expect(chain.mock.results[0].value.deleteTable).not.toHaveBeenCalled();
  });

  it("deletes the surrounding table when the cursor is already in one", () => {
    const { editor, chain } = mockEditor({
      isActive: (name: unknown) => name === "table",
    });
    render(<ScratchpadToolbar editor={editor} />);

    fireEvent.click(screen.getByLabelText("Table"));

    expect(chain.mock.results[0].value.deleteTable).toHaveBeenCalled();
    expect(chain.mock.results[0].value.insertTable).not.toHaveBeenCalled();
  });

  it("hides the add-row/add-column buttons outside a table", () => {
    const { editor } = mockEditor();
    render(<ScratchpadToolbar editor={editor} />);

    expect(screen.queryByLabelText("Add row below")).toBeNull();
    expect(screen.queryByLabelText("Add column right")).toBeNull();
  });

  it("shows and wires the add-row/add-column buttons inside a table", () => {
    const { editor, chain } = mockEditor({
      isActive: (name: unknown) => name === "table",
    });
    render(<ScratchpadToolbar editor={editor} />);

    fireEvent.click(screen.getByLabelText("Add row below"));
    expect(chain.mock.results[0].value.addRowAfter).toHaveBeenCalled();

    fireEvent.click(screen.getByLabelText("Add column right"));
    expect(chain.mock.results[1].value.addColumnAfter).toHaveBeenCalled();
  });

  it("reflects active state at the cursor position", () => {
    const { editor } = mockEditor({
      isActive: (name: unknown) => name === "bold",
    });
    render(<ScratchpadToolbar editor={editor} />);

    expect(screen.getByLabelText("Bold")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByLabelText("Italic")).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });
});
