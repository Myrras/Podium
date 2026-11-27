import type { ElementType, HTMLAttributes } from "react";

import type { ProjectInfo } from "../ipc/types";
import { readableOn } from "../lib/color";
import styles from "./ProjectBadge.module.css";

interface ProjectBadgeProps extends HTMLAttributes<HTMLElement> {
  project: Pick<ProjectInfo, "color" | "iconInitials">;
  /** Edge length in px (default 20). */
  size?: number;
  /** The sidebar renders the badge as a `label` around its colour input. */
  as?: ElementType;
}

/** The project's coloured initials badge (display only). */
export function ProjectBadge({
  project,
  size = 20,
  as: Tag = "span",
  className,
  style,
  children,
  ...rest
}: ProjectBadgeProps) {
  const { color } = project;
  return (
    <Tag
      {...rest}
      className={className ? `${styles.badge} ${className}` : styles.badge}
      style={{
        width: size,
        height: size,
        fontSize: size * 0.45,
        ...(color ? { background: color, color: readableOn(color) } : {}),
        ...style,
      }}
    >
      <span aria-hidden>{project.iconInitials}</span>
      {children}
    </Tag>
  );
}
