/**
 * Collapsible title of a sidebar subsection (To-dos, Agents, …): caret, icon,
 * title and — while collapsed — how many rows it hides, plus pills for how
 * many of those are working and how many need the user. State comes from
 * `useSectionCollapsed`.
 */

import type { ComponentType } from "react";

import { CaretIcon } from "./icons";
import { StatusPill } from "./StatusPill";
import styles from "./Sidebar.module.css";

interface SectionTitleProps {
  title: string;
  Icon: ComponentType<{ className?: string }>;
  count: number;
  /** Hidden rows working / needing the user; only surfaced while collapsed. */
  working?: number;
  needs?: number;
  /** Old name of `needs`, still passed by WorktreeSubsection. */
  waiting?: number;
  collapsed: boolean;
  onToggle: () => void;
}

export function SectionTitle({
  title,
  Icon,
  count,
  working = 0,
  waiting = 0,
  needs = waiting,
  collapsed,
  onToggle,
}: SectionTitleProps) {
  // No aria-label: the visible text ("Terminals (2)") is the accessible name,
  // so a screen reader hears the count too; `aria-expanded` carries the state.
  return (
    <button
      type="button"
      className={styles.sectionToggle}
      aria-expanded={!collapsed}
      title={`${collapsed ? "Expand" : "Collapse"} ${title}`}
      onClick={onToggle}
    >
      <span
        className={styles.caretWrap}
        data-open={!collapsed ? "true" : undefined}
      >
        <CaretIcon size={12} />
      </span>
      <Icon className={styles.panelIcon} />
      <span className={styles.panelTitle}>
        {title}
        {collapsed && count > 0 && (
          <span className={styles.sectionCount}> ({count})</span>
        )}
      </span>
      {collapsed && needs > 0 && <StatusPill state="needs" count={needs} />}
      {collapsed && working > 0 && (
        <StatusPill state="working" count={working} />
      )}
    </button>
  );
}
