/**
 * Formatting toolbar for the scratchpad editor. Purely a thin dispatcher
 * over the live Tiptap `Editor` instance handed up by `ScratchpadEditor` —
 * every button runs a chained command and reflects whether its mark/node is
 * active at the current cursor position (re-derived on every editor
 * transaction via the `transaction` event, since Tiptap's `isActive` is a
 * point-in-time read, not reactive state).
 */

import type { Editor } from "@tiptap/react";
import { useEffect, useState } from "react";

import {
  AddColumnIcon,
  AddRowIcon,
  BlockquoteIcon,
  BoldIcon,
  BulletListIcon,
  ChecklistIcon,
  CodeBlockIcon,
  Heading1Icon,
  Heading2Icon,
  Heading3Icon,
  HorizontalRuleIcon,
  InlineCodeIcon,
  ItalicIcon,
  LinkFormatIcon,
  OrderedListIcon,
  StrikethroughIcon,
  TableIcon,
} from "./icons";
import styles from "./ScratchpadToolbar.module.css";

interface ToolbarAction {
  label: string;
  Icon: typeof BoldIcon;
  isActive: (editor: Editor) => boolean;
  run: (editor: Editor) => void;
  /** Only meaningful while inside a table (add row/column) — hidden otherwise. */
  isEnabled?: (editor: Editor) => boolean;
}

const ACTIONS: ToolbarAction[] = [
  {
    label: "Bold",
    Icon: BoldIcon,
    isActive: (e) => e.isActive("bold"),
    run: (e) => e.chain().focus().toggleBold().run(),
  },
  {
    label: "Italic",
    Icon: ItalicIcon,
    isActive: (e) => e.isActive("italic"),
    run: (e) => e.chain().focus().toggleItalic().run(),
  },
  {
    label: "Strikethrough",
    Icon: StrikethroughIcon,
    isActive: (e) => e.isActive("strike"),
    run: (e) => e.chain().focus().toggleStrike().run(),
  },
  {
    label: "Inline code",
    Icon: InlineCodeIcon,
    isActive: (e) => e.isActive("code"),
    run: (e) => e.chain().focus().toggleCode().run(),
  },
  {
    label: "Link",
    Icon: LinkFormatIcon,
    isActive: (e) => e.isActive("link"),
    run: (e) => {
      const previous: string = e.getAttributes("link").href ?? "";
      const url = window.prompt("Link URL", previous);
      if (url === null) return;
      if (url === "") {
        e.chain().focus().extendMarkRange("link").unsetLink().run();
        return;
      }
      e.chain().focus().extendMarkRange("link").setLink({ href: url }).run();
    },
  },
  {
    label: "Heading 1",
    Icon: Heading1Icon,
    isActive: (e) => e.isActive("heading", { level: 1 }),
    run: (e) => e.chain().focus().toggleHeading({ level: 1 }).run(),
  },
  {
    label: "Heading 2",
    Icon: Heading2Icon,
    isActive: (e) => e.isActive("heading", { level: 2 }),
    run: (e) => e.chain().focus().toggleHeading({ level: 2 }).run(),
  },
  {
    label: "Heading 3",
    Icon: Heading3Icon,
    isActive: (e) => e.isActive("heading", { level: 3 }),
    run: (e) => e.chain().focus().toggleHeading({ level: 3 }).run(),
  },
  {
    label: "Bullet list",
    Icon: BulletListIcon,
    isActive: (e) => e.isActive("bulletList"),
    run: (e) => e.chain().focus().toggleBulletList().run(),
  },
  {
    label: "Numbered list",
    Icon: OrderedListIcon,
    isActive: (e) => e.isActive("orderedList"),
    run: (e) => e.chain().focus().toggleOrderedList().run(),
  },
  {
    label: "Checklist",
    Icon: ChecklistIcon,
    isActive: (e) => e.isActive("taskList"),
    run: (e) => e.chain().focus().toggleTaskList().run(),
  },
  {
    label: "Blockquote",
    Icon: BlockquoteIcon,
    isActive: (e) => e.isActive("blockquote"),
    run: (e) => e.chain().focus().toggleBlockquote().run(),
  },
  {
    label: "Code block",
    Icon: CodeBlockIcon,
    isActive: (e) => e.isActive("codeBlock"),
    run: (e) => e.chain().focus().toggleCodeBlock().run(),
  },
  {
    label: "Horizontal rule",
    Icon: HorizontalRuleIcon,
    isActive: () => false,
    run: (e) => e.chain().focus().setHorizontalRule().run(),
  },
  {
    label: "Table",
    Icon: TableIcon,
    isActive: (e) => e.isActive("table"),
    // Toggle: insert a starter 3x3 table when the cursor isn't in one
    // already, delete the surrounding table when it is — cell-by-cell
    // editing (add/remove row/column) isn't toolbar-driven, matching the
    // rest of the toolbar's "one button per block type" scope.
    run: (e) =>
      e.isActive("table")
        ? e.chain().focus().deleteTable().run()
        : e
            .chain()
            .focus()
            .insertTable({ rows: 3, cols: 3, withHeaderRow: true })
            .run(),
  },
  {
    // Pressing Tab in the last cell already adds a row (built into
    // @tiptap/extension-table's default keymap) — this button is for
    // inserting one anywhere else, or for people who don't know the
    // shortcut. Only shown while inside a table; there's no "active" state
    // for it (it's an action, not a toggle).
    label: "Add row below",
    Icon: AddRowIcon,
    isActive: () => false,
    isEnabled: (e) => e.isActive("table"),
    run: (e) => e.chain().focus().addRowAfter().run(),
  },
  {
    label: "Add column right",
    Icon: AddColumnIcon,
    isActive: () => false,
    isEnabled: (e) => e.isActive("table"),
    run: (e) => e.chain().focus().addColumnAfter().run(),
  },
];

/** Re-render on every editor transaction so active-state buttons stay in sync. */
function useEditorVersion(editor: Editor | null) {
  const [, setVersion] = useState(0);
  useEffect(() => {
    if (!editor) return;
    const bump = () => setVersion((v) => v + 1);
    editor.on("transaction", bump);
    editor.on("selectionUpdate", bump);
    return () => {
      editor.off("transaction", bump);
      editor.off("selectionUpdate", bump);
    };
  }, [editor]);
}

export function ScratchpadToolbar({ editor }: { editor: Editor | null }) {
  useEditorVersion(editor);

  return (
    <div className={styles.toolbar} role="toolbar" aria-label="Formatting">
      {ACTIONS.filter(({ isEnabled }) => !editor || !isEnabled || isEnabled(editor)).map(
        ({ label, Icon, isActive, run }) => (
          <button
            key={label}
            type="button"
            className={styles.button}
            aria-label={label}
            aria-pressed={editor ? isActive(editor) : false}
            data-active={editor ? isActive(editor) : false}
            disabled={!editor}
            onClick={() => editor && run(editor)}
          >
            <Icon />
          </button>
        ),
      )}
    </div>
  );
}
