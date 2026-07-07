/**
 * Heuristic detection of "agent is waiting for input/permission".
 *
 * Coding-agent CLIs (Claude Code, Auggie, …) don't emit structured events
 * over the PTY — when they stall on a permission prompt or a question, the
 * only signal is what's drawn on the terminal. This module holds the pure
 * pattern-matching over that on-screen text so it stays unit-testable; the
 * terminal registry supplies the current viewport text (see
 * `readViewportText`) and the activity store only calls this while the agent
 * is otherwise quiet (no recent output), which keeps false positives low.
 */

/**
 * Patterns that indicate the terminal is showing a confirmation/permission
 * prompt or an input request. Kept conservative — matched only against a
 * quiet screen — so ordinary log output does not trip them.
 */
const PROMPT_PATTERNS: RegExp[] = [
  // Claude Code / Auggie permission questions.
  /do you want to (proceed|continue|make|create|run|allow|apply|delete)/i,
  /would you like to (proceed|continue)/i,
  // Numbered choice menus render a "❯" pointer on the selected option.
  /❯\s*\d+\.\s/,
  /\b1\.\s*yes\b/i,
  /no, and tell (claude|auggie|the agent|codex)/i,
  // Classic yes/no confirmations.
  /\(y\/n\)/i,
  /\[y\/n\]/i,
  /\[y\/n\/a\]/i,
  /\by\s*\/\s*n\b/i,
  // Explicit input / continue requests.
  /press enter to (continue|confirm)/i,
  /waiting for (your )?(input|confirmation|response|approval)/i,
  /permission (to|required|needed|request)/i,
  /awaiting (your )?(input|confirmation|approval)/i,
];

/**
 * Whether the given on-screen terminal text looks like the agent is waiting
 * on the user. `text` should be the current viewport (a screenful), not the
 * whole scrollback; only its tail is examined so a stale prompt scrolled off
 * the top no longer counts.
 */
export function detectInputPrompt(text: string): boolean {
  if (!text) return false;
  const tail = text.length > 4000 ? text.slice(-4000) : text;
  return PROMPT_PATTERNS.some((re) => re.test(tail));
}
