//! Podium speaking first: the watcher that turns things happening in the
//! workspace into nudges for the personal assistant.
//!
//! It subscribes to the [`EventBus`](crate::events::EventBus) and, on a slow
//! tick, looks at processes and to-dos:
//!
//! - an agent exited — finished or crashed;
//! - a running agent has been silent long enough to look stuck on a question;
//! - that same agent *still* blocked a while later — raised once more, with
//!   wording that asks the assistant to propose an answer instead of relaying
//!   the question a second time. That is the last word on that question: two
//!   raises at most, then silence, until the user types into the agent's
//!   terminal (which is a new question cycle);
//! - to-dos that have been open for a long time (a periodic sweep).
//!
//! The agent the user has open on screen ([`Orchestrator::viewing`]) is never
//! announced: they are looking at it. Its blocked question is recorded as
//! already raised, so leaving the agent does not fire a nudge at once.
//!
//! Each of those becomes a `[Podium] …` nudge. Nudges are gathered for a
//! debounce window and triaged on Haiku first ([`crate::triage`]): only the
//! ones worth a turn reach the assistant, together as one turn, which it
//! answers in the chat panel like it would to the user. A nudge **starts the
//! session** when there is none ([`crate::assistant::AssistantManager::nudge`]):
//! the fast lane is event-driven, so a quiet day costs nothing, but when
//! something does happen the user hears about it then — not whenever they next
//! happen to type.
//!
//! There is a second, slower lane: every `poll_interval_minutes` (default 10,
//! 0 = off) the user's enabled external sources — Slack, Microsoft Teams —
//! are checked for anything that needs them. The check runs on Haiku in a
//! session of its own ([`crate::sweep`]), not in the assistant's: it reaches
//! those sources through the user's *own* MCP servers; Podium contributes the
//! timer and the wording, no integration. A sweep that found nothing answers
//! with the [`NOTHING_TO_REPORT`](crate::assistant::NOTHING_TO_REPORT)
//! sentinel and the assistant never hears of it; only a real finding becomes
//! an assistant turn. One sweep runs at a time: a tick while one still runs
//! is skipped.
//!
//! The whole thing is silent while the `proactive` setting is off — the switch
//! is checked per nudge, so flipping it takes effect immediately. Likewise the
//! poll settings are read on every tick, so a changed interval applies without
//! a restart.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use tokio::sync::broadcast::error::RecvError;

use crate::assistant::{poll_prompt, AssistantPolling};
use crate::events::PodiumEvent;
use crate::ids::ProcessId;
use crate::orchestrator::Orchestrator;
use crate::process::{ProcessInfo, ProcessKind, ProcessStatus};
use crate::sweep::SweepOutcome;
use crate::triage::TriageItem;

/// How often one blocked question is raised with the assistant: the first
/// nudge plus one escalation. Raising it every `escalate_after` for as long as
/// the agent lived produced a "no change" reply every ten minutes for hours.
const MAX_RAISES: u32 = 2;

/// How many lingering to-dos are named in a nudge before it just counts them.
const MAX_LINGERING_NAMED: usize = 5;

/// Timings of the watcher. Injectable so tests run in milliseconds instead of
/// hours (same spirit as [`crate::process::supervisor::SupervisorConfig`]).
#[derive(Debug, Clone)]
pub struct ProactivityConfig {
    /// How often the idle-agent check and the sweep clock run.
    pub tick: Duration,
    /// Silence after which a running agent is assumed to be waiting on input.
    pub idle_after: Duration,
    /// How long a flagged agent may stay blocked before Podium raises it
    /// once more — and then asks the assistant to propose an answer rather
    /// than repeat the question. There is no third raise: see
    /// [`MAX_RAISES`].
    pub escalate_after: Duration,
    /// How old an open to-do must be to count as lingering.
    pub lingering_after: Duration,
    /// How often the lingering-to-do sweep runs.
    pub sweep_every: Duration,
    /// What one "minute" of `poll_interval_minutes` is worth. Real time in
    /// the app; milliseconds in tests, so the slow lane can be exercised
    /// without waiting for it.
    pub poll_unit: Duration,
    /// How long nudges are held and gathered before the batch is triaged
    /// ([`crate::triage`]) — the debounce window, counted from the first
    /// nudge of a batch.
    pub triage_window: Duration,
}

impl Default for ProactivityConfig {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(30),
            // Long enough that a thinking agent is not "stuck": Claude Code
            // prints tokens as it works, so real silence means a prompt.
            idle_after: Duration::from_secs(5 * 60),
            // Long enough that the user has had a real chance to answer the
            // first nudge, short enough that a forgotten agent does not sit
            // there all afternoon.
            escalate_after: Duration::from_secs(10 * 60),
            lingering_after: Duration::from_secs(7 * 24 * 60 * 60),
            sweep_every: Duration::from_secs(6 * 60 * 60),
            poll_unit: Duration::from_secs(60),
            triage_window: Duration::from_secs(60),
        }
    }
}

/// Start watching. The task holds only a weak reference, so it ends by
/// itself when the app drops the orchestrator.
pub fn spawn(orchestrator: &Arc<Orchestrator>, config: ProactivityConfig) {
    let weak = Arc::downgrade(orchestrator);
    let mut events = orchestrator.subscribe_events();
    let mut watcher = Watcher {
        weak: weak.clone(),
        config,
        waiting: HashMap::new(),
        // The first sweep comes a whole interval after startup, not the
        // moment the app opens. Same for the external poll.
        last_sweep: Instant::now(),
        last_poll: Instant::now(),
    };
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(watcher.config.tick);
        loop {
            let event = tokio::select! {
                received = events.recv() => match received {
                    Ok(event) => Some(event),
                    // Lagging means we missed events, not that we should stop.
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                },
                _ = ticker.tick() => None,
            };
            let Some(orchestrator) = weak.upgrade() else {
                return;
            };
            match event {
                Some(event) => watcher.on_event(&orchestrator, event).await,
                None => watcher.on_tick(&orchestrator).await,
            }
        }
    });
}

/// What the assistant is told of a finished sweep, worded for `polling` (the
/// sources enabled when it is handed over); `None` for a quiet one.
fn sweep_notice(polling: AssistantPolling, outcome: SweepOutcome) -> Option<String> {
    match outcome {
        SweepOutcome::Quiet => None,
        SweepOutcome::Finding(found) => Some(crate::sweep::finding_text(polling, &found)),
        // A check that keeps failing looks exactly like a quiet one, so it is
        // raised once (the sweep session counts the streak).
        SweepOutcome::NotWorking(reason) => Some(crate::sweep::not_working_text(polling, reason)),
    }
}

struct Watcher {
    /// For the external sweep's own task, which outlives a tick.
    weak: Weak<Orchestrator>,
    config: ProactivityConfig,
    /// Agents already flagged as possibly waiting, so one stuck agent is not
    /// mentioned on every tick. Kept for as long as the agent lives — *not*
    /// only while it looks stuck — because an agent that prints one line and
    /// falls silent again is still waiting on the same unanswered question,
    /// and re-reporting it from scratch is exactly the repetition the
    /// escalation exists to replace. The one thing that ends a record early
    /// is the user typing into the agent after the last raise: they answered,
    /// so whatever blocks it next is a new question.
    waiting: HashMap<ProcessId, Waiting>,
    last_sweep: Instant,
    last_poll: Instant,
}

/// What Podium has already said about one blocked agent.
struct Waiting {
    /// How many times this question has been raised with the assistant (at
    /// most [`MAX_RAISES`]; the agent being viewed counts as one).
    nudges: u32,
    /// When it was last raised, so the next one waits out `escalate_after`.
    last: Instant,
}

impl Watcher {
    /// Put `text` in the triage buffer, unless the user switched proactivity
    /// off. It does not reach the assistant directly: it waits out the
    /// debounce window with whatever else comes up, and the batch is triaged
    /// on Haiku ([`crate::triage`]) so only what is worth a turn is
    /// delivered — as one turn. Use [`nudge_with`](Self::nudge_with) when the
    /// nudge is about a specific agent.
    fn nudge(&self, orchestrator: &Orchestrator, text: String) {
        self.nudge_with(
            orchestrator,
            TriageItem {
                text,
                ..TriageItem::default()
            },
        );
    }

    /// Like [`nudge`](Self::nudge), but takes the whole [`TriageItem`]; it
    /// carries the agent it is about, so the triage's worker can
    /// re-check at delivery that it is still worth saying.
    fn nudge_with(&self, orchestrator: &Orchestrator, item: TriageItem) {
        if !orchestrator.assistant_proactivity() || !orchestrator.assistant_enabled() {
            return;
        }
        if orchestrator.push_triage(item) {
            crate::triage::start_worker(self.weak.clone(), self.config.triage_window);
        }
    }

    /// Run the external sweep in its own session, unless the user switched
    /// proactivity off, and hand the assistant what it found — if anything.
    /// It runs on its own task (a sweep may take minutes), one at a time: a
    /// tick while one still runs is skipped.
    fn poll(&self, orchestrator: &Orchestrator, polling: AssistantPolling) {
        if !orchestrator.assistant_proactivity() || !orchestrator.assistant_enabled() {
            return;
        }
        let Some(sweep) = orchestrator.begin_external_sweep() else {
            return;
        };
        let weak = self.weak.clone();
        tokio::spawn(async move {
            let outcome = sweep.run(&poll_prompt(polling)).await;
            let not_working = matches!(outcome, SweepOutcome::NotWorking(_));
            let mut delivered = false;
            if let Some(orchestrator) = weak.upgrade() {
                // The settings may have changed while the sweep ran: the
                // notice is worded from what is on *now*, so it never names
                // a source the user has since switched off (a finding that
                // came from that source cannot be told apart, and is worded
                // the same).
                let polling_now = orchestrator.assistant_polling();
                if orchestrator.assistant_proactivity() && polling_now.is_active() {
                    if let Some(text) = sweep_notice(polling_now, outcome) {
                        orchestrator.assistant_poll(text).await;
                        delivered = true;
                    }
                }
            }
            // The streak counts its notice as told when it is returned: a
            // dropped one must be raised again, or a check that stays broken
            // would never be mentioned.
            if not_working && !delivered {
                sweep.notice_discarded();
            }
            // Freed only now, so the next sweep never overlaps this one's
            // hand-off.
            drop(sweep);
        });
    }

    async fn on_event(&mut self, orchestrator: &Orchestrator, event: PodiumEvent) {
        let PodiumEvent::ProcessStatusChanged {
            process_id, status, ..
        } = event
        else {
            return;
        };
        let ProcessStatus::Exited {
            code,
            crashed,
            stopped,
            ..
        } = status
        else {
            return;
        };
        // A dead agent cannot be waiting on input any more.
        self.waiting.remove(&process_id);
        // One of the assistant's sub-agents: how it ended is reported by the
        // sub-agent machinery itself (done, failed or stopped), not as an
        // agent that "finished".
        if orchestrator.is_subagent(process_id) {
            return;
        }
        // Someone asked for this stop (the user, or the assistant itself via
        // `stop_process`/`respawn_agent`); "check what it did" would be noise.
        if stopped {
            return;
        }
        let Some(process) = find_agent(orchestrator, process_id) else {
            return;
        };
        // They watched it end.
        if process.viewing {
            return;
        }
        // Nothing below is worth building when no nudge will be sent.
        if !orchestrator.assistant_proactivity() {
            return;
        }
        let where_ = describe(orchestrator, &process);
        if crashed {
            let text = format!(
                "[Podium] The agent {where_} crashed ({}). Look at its last output and tell \
                 the user what went wrong and what it leaves unfinished.",
                exit_code(code)
            );
            self.nudge_with(
                orchestrator,
                TriageItem {
                    text,
                    process_id: Some(process_id),
                    ..TriageItem::default()
                },
            );
            return;
        }
        let text = format!(
            "[Podium] The agent {where_} finished ({}). Check what it did — its to-do and \
             last output — and tell the user in a line or two.",
            exit_code(code)
        );
        // Whether a finished agent left anything for the user is in its last
        // output; the triage has no tools to read it. It is read when the
        // batch is taken (`wants_output`, see `triage::run_worker`), not here:
        // the exit can arrive before the PTY reader has stored the final
        // chunk, and waiting for it would stall this loop once per agent.
        self.nudge_with(
            orchestrator,
            TriageItem {
                text,
                process_id: Some(process_id),
                wants_output: true,
                ..TriageItem::default()
            },
        );
    }

    async fn on_tick(&mut self, orchestrator: &Orchestrator) {
        // While the assistant is off every raise would be dropped by `nudge`
        // / `poll`, yet the watcher would already have spent it (MAX_RAISES,
        // the sweep and poll clocks). Do no work at all, so turning it back
        // on finds everything still due. The clocks keep rolling so the
        // sweeps do not fire in a burst the moment it comes on.
        if !orchestrator.assistant_enabled() {
            self.last_sweep = Instant::now();
            self.last_poll = Instant::now();
            return;
        }
        self.check_idle_agents(orchestrator).await;
        if self.last_sweep.elapsed() >= self.config.sweep_every {
            self.last_sweep = Instant::now();
            self.sweep_lingering_todos(orchestrator).await;
        }
        self.poll_external(orchestrator).await;
    }

    /// The slow lane: every `poll_interval_minutes`, check the enabled
    /// external sources in the sweep's own session. Settings are read here
    /// rather than captured at startup, so turning the poll off (or changing
    /// its interval) takes effect on the next tick.
    async fn poll_external(&mut self, orchestrator: &Orchestrator) {
        let polling = orchestrator.assistant_polling();
        if !polling.is_active() {
            // Keep the clock rolling forward while it is off, so switching it
            // back on waits a full interval instead of firing at once.
            self.last_poll = Instant::now();
            return;
        }
        let every = u32::try_from(polling.interval_minutes)
            .ok()
            .and_then(|minutes| self.config.poll_unit.checked_mul(minutes))
            // An absurd interval is "practically never", not an overflow panic.
            .unwrap_or(Duration::MAX);
        if self.last_poll.elapsed() < every {
            return;
        }
        self.last_poll = Instant::now();
        self.poll(orchestrator, polling);
    }

    /// Flag running agents that need the user: one the UI saw a prompt on
    /// (told at the next tick, not in five minutes), and — as a backstop for
    /// agents whose terminal has never been opened, where there is no screen
    /// to read — one that has simply gone quiet for long enough.
    ///
    /// An agent that is *still* blocked `escalate_after` later is raised
    /// once more, with wording that asks for a decision instead of a repeat of
    /// the question — and then not again ([`MAX_RAISES`]). The agent the user
    /// is viewing is never raised: its question is recorded as seen instead.
    async fn check_idle_agents(&mut self, orchestrator: &Orchestrator) {
        // A prompt actually on screen beats the silence timer for the same
        // agent: one nudge, and the wording that says what is really going on.
        let mut agents: Vec<(ProcessInfo, bool)> = orchestrator
            .agents_awaiting_input()
            .into_iter()
            .map(|p| (p, true))
            .collect();
        let prompting: HashSet<ProcessId> = agents.iter().map(|(p, _)| p.id).collect();
        agents.extend(
            orchestrator
                .idle_agents(self.config.idle_after)
                .into_iter()
                .filter(|p| !prompting.contains(&p.id))
                .map(|p| (p, false)),
        );
        // Forget agents that are gone, but keep the ones merely off the stuck
        // list this tick: their question is still unanswered, so the count
        // has to survive the odd line of output.
        let alive = live_agents(orchestrator);
        self.waiting.retain(|id, _| alive.contains(id));
        // Typing into the agent answers its question: the next time it is
        // blocked, that is a new question and is raised from scratch.
        self.waiting.retain(|id, waiting| {
            orchestrator
                .last_user_input(*id)
                .is_none_or(|typed| typed <= waiting.last)
        });
        for (process, prompting) in agents {
            if process.viewing {
                self.mark_seen(process.id);
                continue;
            }
            // A sub-agent waiting on its own question (ask_assistant) is quiet
            // on purpose, and the assistant already has the question.
            if process
                .subagent
                .as_ref()
                .is_some_and(|s| s.phase == crate::subagent::SubagentPhase::Waiting)
            {
                continue;
            }
            let Some(nudges) = self.next_nudge(process.id) else {
                continue;
            };
            if process.subagent.is_some() {
                let text = self.stuck_subagent_text(&process, nudges, prompting);
                self.nudge_stuck(orchestrator, text, process.id);
                continue;
            }
            let where_ = describe(orchestrator, &process);
            let text = if nudges > 1 {
                format!(
                    "[Podium] The agent {where_} is still blocked on the same question — \
                     raised {nudges} times now and nobody has answered. Do not put the \
                     question to the user again: read its last output, work out what the \
                     answer should be and say which one you would pick and why, as something \
                     they can wave through in a word. You cannot type into an agent's \
                     terminal, so once they have agreed, call respawn_agent: it stops the \
                     stuck session and starts a fresh one carrying that answer."
                )
            } else if prompting {
                format!(
                    "[Podium] The agent {where_} is sitting on a prompt and cannot go on \
                     without the user. Read its last output and tell the user what it is \
                     asking for."
                )
            } else {
                format!(
                    "[Podium] The agent {where_} has printed nothing for {}. It is probably \
                     waiting on input — read its last output and tell the user what it needs.",
                    minutes(self.config.idle_after),
                )
            };
            self.nudge_stuck(orchestrator, text, process.id);
        }
    }

    /// A stuck-agent nudge: only worth saying while the agent still runs and
    /// the user is not looking at it, which the triage re-checks at delivery.
    fn nudge_stuck(&self, orchestrator: &Orchestrator, text: String, id: ProcessId) {
        self.nudge_with(
            orchestrator,
            TriageItem {
                text,
                process_id: Some(id),
                needs_running: true,
                ..TriageItem::default()
            },
        );
    }

    /// What to say about one of the assistant's own sub-agents that looks
    /// stuck: the same three stages as for a project agent, but it is the
    /// assistant's, it has no project, and it cannot be respawned — a fresh
    /// `spawn_subagent` carrying the answer replaces it instead. The user can
    /// also answer it in its terminal, from the sub-agent list.
    fn stuck_subagent_text(&self, process: &ProcessInfo, nudges: u32, prompting: bool) -> String {
        let who = format!(
            "Your sub-agent {} (process {})",
            crate::assistant::quote_line(&process.name),
            process.id
        );
        if nudges > 1 {
            format!(
                "[Podium] {who} is still blocked on the same question — raised {nudges} times \
                 now and nobody has answered. Do not put the question to the user again: read \
                 its last output, work out what the answer should be and say which one you \
                 would pick and why, as something they can wave through in a word. Once they \
                 agree, stop it (stop_process) and start a fresh spawn_subagent whose task \
                 carries that answer."
            )
        } else if prompting {
            format!(
                "[Podium] {who} is sitting on a prompt and cannot go on without the user. \
                 Read its last output and tell the user what it is asking for — they can \
                 answer it in its terminal, from your sub-agent list. (If it is Claude Code's \
                 folder-trust question for Podium's assistant workspace, a folder of Podium's \
                 own, Podium could not accept it ahead of time: the user answers it once, with \
                 \"Yes, I trust this folder\", and later sub-agents do not ask.)"
            )
        } else {
            format!(
                "[Podium] {who} has printed nothing for {}. It is probably waiting on input — \
                 read its last output and tell the user what it needs; they can answer it in \
                 its terminal, from your sub-agent list.",
                minutes(self.config.idle_after),
            )
        }
    }

    /// How many times this question will have been raised if we speak now, or
    /// `None` when it is too soon to say anything about it, or it has been
    /// raised [`MAX_RAISES`] times already.
    fn next_nudge(&mut self, agent: ProcessId) -> Option<u32> {
        let now = Instant::now();
        match self.waiting.get_mut(&agent) {
            None => {
                self.waiting.insert(
                    agent,
                    Waiting {
                        nudges: 1,
                        last: now,
                    },
                );
                Some(1)
            }
            Some(waiting)
                if waiting.nudges < MAX_RAISES
                    && waiting.last.elapsed() >= self.config.escalate_after =>
            {
                waiting.nudges += 1;
                waiting.last = now;
                Some(waiting.nudges)
            }
            Some(_) => None,
        }
    }

    /// The user is looking at this blocked agent, so it needs no telling: the
    /// question counts as raised (once), and a record already there has its
    /// escalation clock restarted — leaving the agent must not nudge at once.
    fn mark_seen(&mut self, agent: ProcessId) {
        let now = Instant::now();
        self.waiting
            .entry(agent)
            .and_modify(|waiting| waiting.last = now)
            .or_insert(Waiting {
                nudges: 1,
                last: now,
            });
    }

    /// Nudge about to-dos nobody has touched in a long time.
    async fn sweep_lingering_todos(&mut self, orchestrator: &Orchestrator) {
        let Ok(max_age) = chrono::Duration::from_std(self.config.lingering_after) else {
            return;
        };
        let cutoff = chrono::Utc::now() - max_age;
        let mut lingering: Vec<String> = Vec::new();
        for project in orchestrator.list_projects() {
            let todos = orchestrator.list_todos(project.id).unwrap_or_default();
            lingering.extend(
                todos
                    .into_iter()
                    .filter(|t| !t.done && !t.archived && t.created_at < cutoff)
                    // Ids, as `describe` gives agents', so the assistant can
                    // key its Responses item to the to-do (`add_response`'s
                    // `todo_id` + `project_id`) and the next sweep replaces
                    // it instead of filing a duplicate.
                    // To-do text and project name can be agent-written: quoted
                    // onto one line (see `quote_line`).
                    .map(|t| {
                        format!(
                            "{} (to-do {}) in project {} (project {})",
                            crate::assistant::quote_line(&t.text),
                            t.id,
                            crate::assistant::quote_line(&project.name),
                            project.id
                        )
                    }),
            );
        }
        if lingering.is_empty() {
            return;
        }
        let named = lingering
            .iter()
            .take(MAX_LINGERING_NAMED)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        let rest = lingering.len().saturating_sub(MAX_LINGERING_NAMED);
        let more = if rest > 0 {
            format!(" and {rest} more")
        } else {
            String::new()
        };
        self.nudge(
            orchestrator,
            format!(
                "[Podium] {} to-do(s) have been open for over {}: {named}{more}. Ask the user \
                 whether to drop them, or to put an agent on one.",
                lingering.len(),
                days(self.config.lingering_after),
            ),
        );
    }
}

/// Ids of every agent process that is still running — the assistant's
/// sub-agents included, whose stuck reports must not restart from scratch.
fn live_agents(orchestrator: &Orchestrator) -> HashSet<ProcessId> {
    orchestrator
        .processes_with_subagents()
        .into_iter()
        .filter(|p| matches!(p.kind, ProcessKind::Agent { .. }))
        .filter(|p| matches!(p.status, ProcessStatus::Running { .. }))
        .map(|p| p.id)
        .collect()
}

/// How much of an agent's output is read for the triage, and how many of its
/// last lines are kept.
const LAST_OUTPUT_BYTES: usize = 8 * 1024;
const LAST_OUTPUT_LINES: usize = 30;

/// The ANSI-stripped last lines an agent printed, for the triage to judge a
/// "finished" nudge by. `None` when it printed nothing (or is gone).
pub(crate) async fn last_output(orchestrator: &Orchestrator, id: ProcessId) -> Option<String> {
    let raw = orchestrator.tail_text(id, LAST_OUTPUT_BYTES).await.ok()?;
    let plain = String::from_utf8_lossy(&strip_ansi_escapes::strip(raw.as_bytes())).into_owned();
    let lines: Vec<&str> = plain
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(LAST_OUTPUT_LINES);
    let tail = lines[start..].join("\n");
    // Bounded by characters too: a few very long lines (a TUI repaint) would
    // otherwise ride along whole, and it is the end that matters.
    (!tail.is_empty())
        .then(|| crate::triage::truncate_tail(&tail, crate::triage::MAX_CONTEXT_CHARS))
}

/// The agent process with this id, or `None` when it is gone or not an agent.
fn find_agent(orchestrator: &Orchestrator, id: ProcessId) -> Option<ProcessInfo> {
    orchestrator
        .list_processes(None)
        .into_iter()
        .find(|p| p.id == id && matches!(p.kind, ProcessKind::Agent { .. }))
}

/// `"name" in project "project"` — how a nudge points at an agent.
fn describe(orchestrator: &Orchestrator, process: &ProcessInfo) -> String {
    let project = orchestrator
        .list_projects()
        .into_iter()
        .find(|p| p.id == process.project_id)
        .map(|p| p.name)
        .unwrap_or_else(|| "a closed project".to_string());
    // The process id lets the assistant file the update as a Responses item
    // keyed to this agent (`add_response`'s `process_id`), so a later update
    // replaces it instead of piling up. The agent names itself
    // (`rename_session`), so its name is quoted onto one line, like the
    // project's (see `quote_line`).
    format!(
        "{} (process {}) in project {}",
        crate::assistant::quote_line(&process.name),
        process.id,
        crate::assistant::quote_line(&project)
    )
}

fn exit_code(code: Option<i32>) -> String {
    match code {
        Some(code) => format!("exit code {code}"),
        None => "no exit code".to_string(),
    }
}

fn minutes(d: Duration) -> String {
    let minutes = d.as_secs() / 60;
    if minutes == 0 {
        format!("{} seconds", d.as_secs().max(1))
    } else {
        format!("{minutes} minutes")
    }
}

fn days(d: Duration) -> String {
    let days = d.as_secs() / (24 * 60 * 60);
    if days == 0 {
        format!("{} hours", (d.as_secs() / 3600).max(1))
    } else {
        format!("{days} days")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sweep_notice_names_only_the_sources_enabled_when_it_is_handed_over() {
        let both = AssistantPolling {
            interval_minutes: 10,
            slack: true,
            teams: true,
        };
        let slack_only = AssistantPolling {
            teams: false,
            ..both
        };
        let finding = || SweepOutcome::Finding("Ada asks for the Q3 numbers".to_string());
        let text = sweep_notice(slack_only, finding()).expect("a finding");
        assert!(text.contains("routine check of Slack found"), "{text}");
        assert!(!text.contains("Teams"), "{text}");
        assert_eq!(
            sweep_notice(both, finding()).as_deref(),
            Some(crate::sweep::finding_text(both, "Ada asks for the Q3 numbers").as_str())
        );
        let down = sweep_notice(slack_only, SweepOutcome::NotWorking("it took too long"))
            .expect("a notice");
        assert!(
            down.contains("routine check of Slack is not working"),
            "{down}"
        );
        assert!(!down.contains("Slack and Microsoft Teams"), "{down}");
        assert_eq!(sweep_notice(both, SweepOutcome::Quiet), None);
    }

    #[test]
    fn durations_read_as_prose() {
        assert_eq!(minutes(Duration::from_secs(300)), "5 minutes");
        assert_eq!(minutes(Duration::from_millis(200)), "1 seconds");
        assert_eq!(days(Duration::from_secs(7 * 24 * 3600)), "7 days");
        assert_eq!(days(Duration::from_secs(3 * 3600)), "3 hours");
    }

    #[test]
    fn an_exit_without_a_code_is_still_reported() {
        assert_eq!(exit_code(Some(0)), "exit code 0");
        assert_eq!(exit_code(None), "no exit code");
    }
}
