/**
 * Small header button that copies an item's id (to-do or scratchpad) to the
 * clipboard, briefly flashing a check on success. Shared by the to-do and
 * scratchpad detail panes; it borrows the host pane's icon-button class via
 * `className` so it needs no CSS of its own.
 */

import { useState } from "react";

import { toastError } from "../state/toastStore";
import { CheckIcon, CopyIcon } from "./icons";

export function CopyIdButton({
  id,
  className,
}: {
  id: string;
  className?: string;
}) {
  const [copied, setCopied] = useState(false);

  const copy = () => {
    navigator.clipboard
      .writeText(id)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1200);
      })
      .catch(() => toastError("Failed to copy id to clipboard"));
  };

  return (
    <button
      type="button"
      className={className}
      aria-label="Copy id"
      title={copied ? "Copied!" : `Copy id: ${id}`}
      onClick={copy}
    >
      {copied ? <CheckIcon size={12} /> : <CopyIcon size={12} />}
    </button>
  );
}
