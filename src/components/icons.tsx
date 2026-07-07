/**
 * Central icon set for Podium.
 *
 * The rest of the app imports semantic names from here and never references
 * `lucide-react` directly, so re-theming a glyph is a one-line change in this
 * file.
 *
 * Defaults: 14px at stroke-width 1.75 reads crisp and light at our small UI
 * sizes (lucide's own defaults are 24 / 2, too heavy here). Icons are purely
 * decorative — the buttons and rows that hold them already carry the
 * `aria-label`/`title` — so they default to `aria-hidden`. Every prop stays
 * overridable per use (size, strokeWidth, color, className…); colour follows
 * `currentColor`, so icons inherit the active theme with no per-theme work.
 */

import {
  Archive,
  ArchiveRestore,
  Bot,
  Link,
  ListTodo,
  MessageSquare,
  SquareTerminal,
  Play,
  Square,
  RotateCw,
  Plus,
  Folder,
  FolderOpen,
  Settings,
  Check,
  X,
  Minus,
  Sun,
  Moon,
  Monitor,
  Copy,
  ChevronRight,
  CircleAlert,
  CircleCheck,
  Info,
  Pencil,
  Trash2,
  GripVertical,
  type LucideIcon,
  type LucideProps,
} from "lucide-react";

/** Wrap a lucide glyph with our shared defaults; `defaults` then `props` win. */
function icon(Glyph: LucideIcon, defaults?: Partial<LucideProps>) {
  function Icon(props: LucideProps) {
    return (
      <Glyph
        size={14}
        strokeWidth={1.75}
        aria-hidden={true}
        {...defaults}
        {...props}
      />
    );
  }
  Icon.displayName = `Icon(${Glyph.displayName ?? "lucide"})`;
  return Icon;
}

// Sidebar entities: agents, spawned processes, terminals, to-dos, projects.
export const AgentIcon = icon(Bot);
export const TerminalIcon = icon(SquareTerminal);
export const TodoIcon = icon(ListTodo);
export const FolderIcon = icon(Folder);
export const FolderOpenIcon = icon(FolderOpen);

// Actions / affordances.
export const AddIcon = icon(Plus, { size: 15 });
export const MinusIcon = icon(Minus, { size: 15 });
export const CloseIcon = icon(X);
export const RunIcon = icon(Play);
export const StopIcon = icon(Square);
export const RestartIcon = icon(RotateCw);
export const CheckIcon = icon(Check);
export const CopyIcon = icon(Copy);
export const CaretIcon = icon(ChevronRight); // rotates 90° on expand (CSS)
export const CommentIcon = icon(MessageSquare);
export const LinkIcon = icon(Link, { size: 13 });
export const ArchiveIcon = icon(Archive, { size: 13 });
export const UnarchiveIcon = icon(ArchiveRestore, { size: 13 });
export const EditIcon = icon(Pencil, { size: 13 });
export const DeleteIcon = icon(Trash2, { size: 13 });
export const GripIcon = icon(GripVertical, { size: 14 });

// Theme glyphs (dark / light / system).
export const SunIcon = icon(Sun);
export const MoonIcon = icon(Moon);
export const MonitorIcon = icon(Monitor);

// App chrome (Windows-only title bar). Window controls use a single line
// (minimize), a square (maximize), two stacked squares (restore), and the
// shared X (close) to match the platform convention.
export const SettingsIcon = icon(Settings);
export const WindowMinimizeIcon = icon(Minus, { size: 16 });
export const WindowMaximizeIcon = icon(Square, { size: 12 });
export const WindowRestoreIcon = icon(Copy, { size: 12 });

// Toast severity.
export const ErrorIcon = icon(CircleAlert, { size: 16 });
export const SuccessIcon = icon(CircleCheck, { size: 16 });
export const InfoIcon = icon(Info, { size: 16 });
