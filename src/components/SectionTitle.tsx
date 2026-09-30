/**
 * Collapsible title of a sidebar subsection (To-dos, Agents, …): caret, icon,
 * title and — while collapsed — how many rows it hides. State comes from
 * `useSectionCollapsed`.
 */

import type { ComponentType } from "react";

import { CaretIcon } from "./icons";
import styles from "./Sidebar.module.css";

interface SectionTitleProps {
  title: string;
  Icon: ComponentType<{ className?: string }>;
  count: number;
  collapsed: boolean;
  onToggle: () => void;
}

export function SectionTitle({
  title,
  Icon,
  count,
  collapsed,
  onToggle,
}: SectionTitleProps) {
  return (
    <button
      type="button"
      className={styles.sectionToggle}
      aria-expanded={!collapsed}
      aria-label={`${collapsed ? "Expand" : "Collapse"} ${title}`}
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
    </button>
  );
}
