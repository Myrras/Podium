/**
 * Heuristics read off a process's terminal screen: "waiting for
 * input/permission", and how many background shells an agent has running.
 *
 * Neither coding-agent CLIs (Claude Code, Auggie, …) nor ordinary commands
 * emit structured events over the PTY — when they stall on a permission
 * prompt, a question, or a `sudo` password, the only signal is what's drawn
 * on the terminal. This module holds the pure
 * pattern-matching over that on-screen text so it stays unit-testable; the
 * terminal registry supplies the current viewport text (see
 * `readViewportText`) and the activity store only asks about a prompt while
 * the agent is otherwise quiet (no recent output), which keeps false
 * positives low. The shell count is read every tick — a background shell is
 * just as interesting while the agent talks as while it sits idle.
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
  // Credential prompts from ordinary commands run in a terminal: sudo
  // ("[sudo] password for x:", plain "Password:" on macOS), ssh
  // ("(user@host) Password:", "user@host's password:"), key passphrases, git
  // credentials. Anchored only to the END of the line — that is where the
  // cursor sits, while the start is whatever host/user the tool prefixes —
  // so a log line merely mentioning a password does not trip them.
  /\bpasswords?( for [^\n:]*)?:[ \t]*$/im,
  /\bpassphrase( for [^\n:]*)?:[ \t]*$/im,
  /\busername for [^\n]*:[ \t]*$/im,
];

/**
 * Whether the given on-screen terminal text looks like the process is waiting
 * on the user. `text` should be the current viewport (a screenful), not the
 * whole scrollback; only its tail is examined so a stale prompt scrolled off
 * the top no longer counts.
 */
export function detectInputPrompt(text: string): boolean {
  if (!text) return false;
  const tail = text.length > 4000 ? text.slice(-4000) : text;
  return PROMPT_PATTERNS.some((re) => re.test(tail));
}

/**
 * Background-shell marker in Claude Code's footer: while background bash
 * tasks are alive it renders "· 1 shell ·" / "· 2 shells ·" beside the mode
 * hint, with a "↓ to manage" affordance. The count must be a whole footer
 * segment — middle dot before it, another one (or the line end) after — so
 * neither ordinary output mentioning shells nor Claude Code's own "Ran 2
 * shell commands" tool summary can be mistaken for it.
 */
const BACKGROUND_SHELLS = /·\s*(\d+)\s+shells?(?=\s*(?:·|$))/m;

/**
 * How many background shells the agent's screen reports (0 when none). This
 * is what makes an "idle" agent legible: it can be done talking to you and
 * still have a build or test run going behind the prompt.
 */
export function detectBackgroundShells(text: string): number {
  if (!text) return 0;
  const tail = text.length > 4000 ? text.slice(-4000) : text;
  const match = BACKGROUND_SHELLS.exec(tail);
  return match ? Number(match[1]) : 0;
}
