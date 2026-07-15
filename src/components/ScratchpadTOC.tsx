/**
 * "On this page" outline panel for a scratchpad: the current document's
 * H1/H2/H3 headings, nested by level, in a scrollable side panel (H1 and H2
 * render at the same unindented tier — see `Heading.level`'s doc comment).
 * Purely presentational — `ScratchpadDetailPane` supplies the live
 * `Heading[]` (see `../lib/tiptapHeadings`) and a click handler that scrolls
 * the editor.
 */

import type { Heading } from "../lib/tiptapHeadings";
import styles from "./ScratchpadTOC.module.css";

export function ScratchpadTOC({
  headings,
  onSelectHeading,
}: {
  headings: Heading[];
  onSelectHeading: (heading: Heading) => void;
}) {
  return (
    <nav className={styles.toc} aria-label="On this page">
      <div className={styles.title}>On this page</div>
      <div className={styles.scroll}>
        {headings.length === 0 ? (
          <p className={styles.empty}>No headings yet</p>
        ) : (
          <ul className={styles.list}>
            {headings.map((heading) => (
              <li
                key={heading.pos}
                className={styles.item}
                data-level={heading.level}
              >
                <button
                  type="button"
                  className={styles.link}
                  title={heading.text}
                  onClick={() => onSelectHeading(heading)}
                >
                  {heading.text || "Untitled heading"}
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </nav>
  );
}
