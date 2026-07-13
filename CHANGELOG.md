# Changelog

All notable **functional** (user-facing) changes to Podium are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Internal refactors, tooling, and chores are intentionally omitted. (Automated
changelog generation from Conventional Commits via git-cliff is planned but not
yet set up.)

## [1.1.0] - 2026-07-13

### Added

- **Notifications now play a sound.** When an agent needs your input, Podium
  plays a sound alongside the OS notification and toast — a built-in beep by
  default, or a custom audio file you pick. A new Settings → General →
  Notifications section adds a Play sound toggle, a Choose…/Reset custom-sound
  picker, and a Test button.

## [1.0.0] - 2026-07-07

### Added

- **Open a project and run its dev servers, terminals, and AI agents in one
  cockpit.** Point Podium at a project folder and it reads `podium.yml`,
  starting each configured service, ad-hoc terminal, and coding agent in its
  own PTY with a real terminal view, lifecycle supervision, and a sidebar to
  switch between them. Opened projects persist as a reorderable, renamable
  workspace and reopen on the next launch.
- **Real terminals that survive tab switches.** Each process gets an xterm.js
  view with full scrollback that is retained when you move between tabs, plus
  input, resize, and a live "working…"/idle activity indicator for agents.
- **Supervised processes.** Services can auto-start and auto-restart
  (`never` / `on-crash` / `always`) with exponential backoff and a circuit
  breaker; a manual stop is never treated as a crash and kills the whole
  process group so children can't outlive it.
- **AI coding agents, including agents that spawn agents.** Launch Claude Code
  or Auggie from the new-agent modal (up to 8 per project). A built-in MCP
  server lets agents inspect and orchestrate the processes Podium manages —
  listing/starting/stopping them, reading output, and spawning more agents.
- **Shared per-project to-dos.** Each project has a to-do list visible in the
  sidebar and to every agent over MCP, with a detail view for descriptions and
  a comment thread. To-dos can carry pinned issue/PR links, be assigned to a
  running agent, and be archived (done items auto-archive from earlier days).
- **Connect external MCP clients once.** A stdio bridge lets tools like Claude
  Code talk to Podium's MCP server through a stable config even though the
  port and token rotate every launch; the Settings → MCP tab offers one-click
  install and status.
- **Dark/light themes, settings, and toasts**, with a motion system that
  respects reduced-motion preferences.
