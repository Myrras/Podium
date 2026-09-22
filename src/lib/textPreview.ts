/**
 * One-line preview of user- or agent-authored text for native `title`
 * tooltips.
 *
 * To-dos, scratchpads and Podium's own nudges carry free-form text that can
 * run to paragraphs, and a `title` renders it verbatim — newlines and all —
 * which the OS then paints as an unreadable wall next to the cursor. Every
 * tooltip embedding such text goes through this first: whitespace (including
 * line breaks) collapses to single spaces and anything past `max` is cut at a
 * word boundary and ellipsized.
 */
export function textPreview(text: string, max = 80): string {
  const flat = text.replace(/\s+/g, " ").trim();
  if (flat.length <= max) return flat;
  const cut = flat.slice(0, max);
  // Prefer breaking at the last space so the preview ends on a whole word,
  // unless that would throw away most of the budget.
  const lastSpace = cut.lastIndexOf(" ");
  return `${(lastSpace > max * 0.6 ? cut.slice(0, lastSpace) : cut).trimEnd()}…`;
}
