/**
 * Collapsible title of a sidebar subsection (To-dos, Agents, …): caret, icon,
 * title and — while collapsed — how many rows it hides, plus a "needs input"
 * dot when one of those hidden rows is waiting on the user. State comes from
 * `useSectionCollapsed`.
 */

import type { ComponentType } from "react";

import { CaretIcon } from "./icons";
import styles from "./Sidebar.module.css";

interface SectionTitleProps {
  title: string;
  Icon: ComponentType<{ className?: string }>;
  count: number;
  /** Hidden rows waiting on the user; only surfaced while collapsed. */
  waiting?: number;
  collapsed: boolean;
  onToggle: () => void;
}

export function SectionTitle({
  title,
  Icon,
  count,
  waiting = 0,
  collapsed,
  onToggle,
}: SectionTitleProps) {
  const needsInput = collapsed && waiting > 0;
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
      {needsInput && (
        <span
          className={styles.sectionWaiting}
          role="img"
          aria-label={`${waiting} need${waiting === 1 ? "s" : ""} your input`}
          title={`${waiting} need${waiting === 1 ? "s" : ""} your input`}
        />
      )}
    </button>
  );
}
