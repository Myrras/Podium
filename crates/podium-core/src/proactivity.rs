//! Podium speaking first: the watcher that turns things happening in the
//! workspace into nudges for the personal assistant.
//!
//! It subscribes to the [`EventBus`](crate::events::EventBus) and, on a slow
//! tick, looks at processes and to-dos:
//!
//! - an agent exited — finished or crashed;
//! - a running agent has been silent long enough to look stuck on a question;
//! - to-dos that have been open for a long time (a periodic sweep).
//!
//! Each of those becomes one `[Podium] …` turn handed to the assistant, which
//! answers in the chat panel like it would to the user. A nudge **starts the
//! session** when there is none ([`crate::assistant::AssistantManager::nudge`]):
//! the fast lane is event-driven, so a quiet day costs nothing, but when
//! something does happen the user hears about it then — not whenever they next
//! happen to type.
//!
//! There is a second, slower lane: every `poll_interval_minutes` (default 10,
//! 0 = off) the assistant is asked to look at the user's enabled external
//! sources — Slack, Microsoft Teams — and speak only if something needs them.
//! It reaches those through the user's *own* MCP servers, which the assistant
//! inherits; Podium contributes the timer and the wording, no integration. A
//! sweep that found nothing answers with the
//! [`NOTHING_TO_REPORT`](crate::assistant::NOTHING_TO_REPORT) sentinel, which
//! is swallowed before it can become a chat row or a notification.
//!
//! The whole thing is silent while the `proactive` setting is off — the switch
//! is checked per nudge, so flipping it takes effect immediately. Likewise the
//! poll settings are read on every tick, so a changed interval applies without
//! a restart.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::broadcast::error::RecvError;

use crate::assistant::poll_prompt;
use crate::events::PodiumEvent;
use crate::ids::ProcessId;
use crate::orchestrator::Orchestrator;
use crate::process::{ProcessInfo, ProcessKind, ProcessStatus};

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
    /// How old an open to-do must be to count as lingering.
    pub lingering_after: Duration,
    /// How often the lingering-to-do sweep runs.
    pub sweep_every: Duration,
    /// What one "minute" of `poll_interval_minutes` is worth. Real time in
    /// the app; milliseconds in tests, so the slow lane can be exercised
    /// without waiting for it.
    pub poll_unit: Duration,
}

impl Default for ProactivityConfig {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(30),
            // Long enough that a thinking agent is not "stuck": Claude Code
            // prints tokens as it works, so real silence means a prompt.
            idle_after: Duration::from_secs(5 * 60),
            lingering_after: Duration::from_secs(7 * 24 * 60 * 60),
            sweep_every: Duration::from_secs(6 * 60 * 60),
            poll_unit: Duration::from_secs(60),
        }
    }
}

/// Start watching. The task holds only a weak reference, so it ends by
/// itself when the app drops the orchestrator.
pub fn spawn(orchestrator: &Arc<Orchestrator>, config: ProactivityConfig) {
    let weak = Arc::downgrade(orchestrator);
    let mut events = orchestrator.subscribe_events();
    let mut watcher = Watcher {
        config,
        idle_nudged: HashSet::new(),
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

struct Watcher {
    config: ProactivityConfig,
    /// Agents already flagged as possibly waiting, so one stuck agent is
    /// mentioned once and not on every tick. An agent that speaks again (or
    /// exits) drops out and can be flagged afresh.
    idle_nudged: HashSet<ProcessId>,
    last_sweep: Instant,
    last_poll: Instant,
}

impl Watcher {
    /// Hand `text` to the assistant, unless the user switched proactivity off.
    async fn nudge(&self, orchestrator: &Orchestrator, text: String) {
        if !orchestrator.assistant_proactivity() {
            return;
        }
        orchestrator.assistant_nudge(text).await;
    }

    /// Hand the assistant an external-sweep errand, unless the user switched
    /// proactivity off. Same gate as a nudge, but it stays out of the chat.
    async fn poll(&self, orchestrator: &Orchestrator, text: String) {
        if !orchestrator.assistant_proactivity() {
            return;
        }
        orchestrator.assistant_poll(text).await;
    }

    async fn on_event(&mut self, orchestrator: &Orchestrator, event: PodiumEvent) {
        let PodiumEvent::ProcessStatusChanged {
            process_id, status, ..
        } = event
        else {
            return;
        };
        let ProcessStatus::Exited { code, crashed, .. } = status else {
            return;
        };
        // A dead agent cannot be waiting on input any more.
        self.idle_nudged.remove(&process_id);
        let Some(process) = find_agent(orchestrator, process_id) else {
            return;
        };
        let where_ = describe(orchestrator, &process);
        let text = if crashed {
            format!(
                "[Podium] The agent {where_} crashed ({}). Look at its last output and tell \
                 the user what went wrong and what it leaves unfinished.",
                exit_code(code)
            )
        } else {
            format!(
                "[Podium] The agent {where_} finished ({}). Check what it did — its to-do and \
                 last output — and tell the user in a line or two.",
                exit_code(code)
            )
        };
        self.nudge(orchestrator, text).await;
    }

    async fn on_tick(&mut self, orchestrator: &Orchestrator) {
        self.check_idle_agents(orchestrator).await;
        if self.last_sweep.elapsed() >= self.config.sweep_every {
            self.last_sweep = Instant::now();
            self.sweep_lingering_todos(orchestrator).await;
        }
        self.poll_external(orchestrator).await;
    }

    /// The slow lane: every `poll_interval_minutes`, ask the assistant to
    /// check the enabled external sources. Settings are read here rather than
    /// captured at startup, so turning the poll off (or changing its
    /// interval) takes effect on the next tick.
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
        self.poll(orchestrator, poll_prompt(polling)).await;
    }

    /// Flag running agents that need the user: one the UI saw a prompt on
    /// (told at the next tick, not in five minutes), and — as a backstop for
    /// agents whose terminal has never been opened, where there is no screen
    /// to read — one that has simply gone quiet for long enough.
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
        let ids: HashSet<ProcessId> = agents.iter().map(|(p, _)| p.id).collect();
        // An agent that spoke again is no longer stuck, so it may be flagged
        // again later.
        self.idle_nudged.retain(|id| ids.contains(id));
        for (process, prompting) in agents {
            if !self.idle_nudged.insert(process.id) {
                continue;
            }
            let where_ = describe(orchestrator, &process);
            let text = if prompting {
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
            self.nudge(orchestrator, text).await;
        }
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
                    .map(|t| format!("{} ({})", t.text, project.name)),
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
        )
        .await;
    }
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
    format!("\"{}\" in project \"{project}\"", process.name)
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
