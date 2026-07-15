/**
 * Free-text tag chips with an inline "add" input. Used by the scratchpad
 * detail pane; kept generic (plain string tags, no ids) in case other panes
 * grow tags later.
 */

import { useState } from "react";

import { CloseIcon, TagIcon } from "./icons";
import styles from "./TagChip.module.css";

export function TagChip({
  tags,
  onAdd,
  onRemove,
}: {
  tags: string[];
  onAdd: (tag: string) => void;
  onRemove: (tag: string) => void;
}) {
  const [draft, setDraft] = useState("");

  const submit = () => {
    const trimmed = draft.trim();
    if (!trimmed) {
      setDraft("");
      return;
    }
    onAdd(trimmed);
    setDraft("");
  };

  return (
    <div className={styles.row}>
      <TagIcon className={styles.rowIcon} />
      {tags.map((tag) => (
        <span key={tag} className={styles.chip}>
          {tag}
          <button
            type="button"
            className={styles.remove}
            aria-label={`Remove tag ${tag}`}
            title="Remove tag"
            onClick={() => onRemove(tag)}
          >
            <CloseIcon size={10} />
          </button>
        </span>
      ))}
      <input
        className={styles.input}
        value={draft}
        placeholder="Add tag…"
        aria-label="Add tag"
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            submit();
          }
        }}
        onBlur={submit}
      />
    </div>
  );
}
