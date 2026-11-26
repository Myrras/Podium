//! Podium speaking first: the watcher that files agents that need the user
//! on the Responses list, and runs the slow external sweep.
//!
//! It subscribes to the [`EventBus`](crate::events::EventBus) and, on a slow
//! tick, looks at the agents:
//!
//! - an agent waiting on the user (the "needs you" activity state Floor mode
//!   shows: Claude Code's own `waiting` report, else the prompt the UI saw on
//!   screen, [`Orchestrator::agents_awaiting_input`]) becomes an item keyed
//!   `agent:<processId>`, reason `question`. It goes when the agent stops
//!   waiting. The agent the user is viewing gets none: they are looking at it,
//!   and that question stays unfiled after they leave. Opening or typing into
//!   an agent dismisses its item at once (soft, in the orchestrator), so the
//!   same episode re-filed here stays out of sight.
//! - an agent that crashed (not a stop on request) becomes the same key with
//!   reason `crashed`. It goes when the agent is restarted or removed.
//!
//! The fingerprint is `question:<since>` / `crashed:<since>`, so a re-file of
//! the same episode is `Unchanged`: nothing is published, and the frontend's
//! desktop notification (new item ids on the Responses list) fires only for
//! `New`/`Changed`. Nothing here writes to the chat.
//!
//! There is a second, slower lane: every `poll_interval_minutes` (default 10,
//! 0 = off) the user's enabled external sources — Slack, Microsoft Teams —
//! are checked for messages still waiting for the user. The check runs on
//! Haiku in a session of its own ([`crate::sweep`]), not in the assistant's:
//! it reaches those sources through the user's *own* MCP servers. It is
//! stateless: every sweep returns the full set, each becomes an item keyed
//! `msg:<source>:<conversation>:<thread>` (reason `message`, fingerprint the
//! newest message's time), and every `msg:` item not in the set goes. A sweep
//! that failed files and removes nothing. Nothing of it reaches the chat but
//! the one notice when the check keeps failing. One sweep runs at a time: a
//! tick while one still runs is skipped.
//!
//! Agent items are filed, and the external sweep runs, whenever the assistant
//! is on; the `proactive` setting only gates desktop notifications (the
//! frontend's call). The poll settings are read on every tick, so a changed
//! interval or source applies without a restart.

use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::sync::broadcast::error::RecvError;

use crate::activity::ReportedState;
use crate::assistant::{poll_prompt, AssistantPolling};
use crate::events::PodiumEvent;
use crate::ids::ProcessId;
use crate::orchestrator::Orchestrator;
use crate::process::{ProcessKind, ProcessStatus};
use crate::responses::{ResponseKind, ResponseReason};
use crate::sweep::SweepOutcome;

/// Timings of the watcher. Injectable so tests run in milliseconds instead of
/// minutes (same spirit as [`crate::process::supervisor::SupervisorConfig`]).
#[derive(Debug, Clone)]
pub struct ProactivityConfig {
    /// How often the waiting-agent check and the poll clock run.
    pub tick: Duration,
    /// What one "minute" of `poll_interval_minutes` is worth. Real time in
    /// the app; milliseconds in tests, so the slow lane can be exercised
    /// without waiting for it.
    pub poll_unit: Duration,
}

impl Default for ProactivityConfig {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(30),
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
        weak: weak.clone(),
        config,
        asking: HashMap::new(),
        // The first poll comes a whole interval after startup, not the
        // moment the app opens.
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
                Some(event) => watcher.on_event(&orchestrator, event),
                None => watcher.on_tick(&orchestrator).await,
            }
        }
    });
}

/// Whether Podium may file agent items right now: whenever the assistant is
/// on. `proactive` only gates the desktop notification, which the frontend
/// decides, so the list stays complete with it off.
fn active(orchestrator: &Orchestrator) -> bool {
    orchestrator.assistant_enabled()
}

struct Watcher {
    /// For the external sweep's own task, which outlives a tick.
    weak: Weak<Orchestrator>,
    config: ProactivityConfig,
    /// Agents seen waiting on the user, one record per waiting episode.
    asking: HashMap<ProcessId, Asking>,
    last_poll: Instant,
}

/// One waiting episode of an agent.
struct Asking {
    /// When the watcher first saw it waiting: the item's fingerprint, so
    /// every tick of the same episode re-files it as `Unchanged`.
    since: DateTime<Utc>,
    at: Instant,
    /// The user had it on screen: they know, so nothing is filed for this
    /// episode, not even after they leave it.
    seen: bool,
}

impl Asking {
    fn new() -> Self {
        Self {
            since: Utc::now(),
            at: Instant::now(),
            seen: false,
        }
    }
}

impl Watcher {
    /// Run the external sweep in its own session and file what it found on
    /// the Responses list. It
    /// runs on its own task (a sweep may take minutes), one at a time: a
    /// tick while one still runs is skipped.
    fn poll(&self, orchestrator: &Orchestrator, polling: AssistantPolling) {
        if !orchestrator.assistant_enabled() {
            return;
        }
        let Some(sweep) = orchestrator.begin_external_sweep() else {
            return;
        };
        let weak = self.weak.clone();
        tokio::spawn(async move {
            let outcome = sweep.run(&poll_prompt(polling, Utc::now())).await;
            let not_working = matches!(outcome, SweepOutcome::NotWorking(_));
            let mut delivered = false;
            if let Some(orchestrator) = weak.upgrade() {
                // The settings may have changed while the sweep ran: what is
                // filed, and the notice's wording, follow what is on *now*.
                let polling_now = orchestrator.assistant_polling();
                if orchestrator.assistant_enabled() && polling_now.is_active() {
                    match outcome {
                        SweepOutcome::Quiet => {}
                        SweepOutcome::Messages(mut messages) => {
                            // A source switched off since: none of its items.
                            messages.retain(|m| match m.source {
                                "slack" => polling_now.slack,
                                _ => polling_now.teams,
                            });
                            if let Err(e) = orchestrator.sync_message_responses(&messages) {
                                tracing::warn!("could not update the message items: {e}");
                            }
                        }
                        // A check that keeps failing looks exactly like a
                        // quiet one, so it is raised once, in the chat.
                        SweepOutcome::NotWorking(reason) => {
                            let text = crate::sweep::not_working_text(polling_now, reason);
                            orchestrator.assistant_poll(text).await;
                            delivered = true;
                        }
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

    /// An agent exited or was (re)started: file a crash, or drop its item.
    fn on_event(&mut self, orchestrator: &Orchestrator, event: PodiumEvent) {
        let PodiumEvent::ProcessStatusChanged {
            process_id, status, ..
        } = event
        else {
            return;
        };
        let result = match status {
            ProcessStatus::Exited {
                code,
                crashed,
                stopped,
                at,
            } => {
                // A dead agent cannot be waiting on input any more.
                self.asking.remove(&process_id);
                let crash = orchestrator.process_info(process_id).filter(|p| {
                    // Sub-agents: the sub-agent machinery reports how they
                    // ended. A requested stop (the user, `stop_process`,
                    // `respawn_agent`) is no crash. The viewed agent: they
                    // watched it end.
                    crashed
                        && !stopped
                        && matches!(p.kind, ProcessKind::Agent { .. })
                        && p.subagent.is_none()
                        && !p.viewing
                });
                match crash {
                    Some(process) if active(orchestrator) => orchestrator.file_agent_response(
                        &process,
                        ResponseKind::Fyi,
                        ResponseReason::Crashed,
                        format!("crashed:{}", at.timestamp_millis()),
                        format!("Crashed ({}).", exit_code(code)),
                    ),
                    _ => orchestrator.remove_agent_response(process_id),
                }
            }
            // A restart: whatever the item said is over.
            ProcessStatus::Running { .. } => orchestrator.remove_agent_response(process_id),
            _ => Ok(()),
        };
        if let Err(e) = result {
            tracing::warn!("could not update an agent's Responses item: {e}");
        }
    }

    async fn on_tick(&mut self, orchestrator: &Orchestrator) {
        if let Err(e) = self.check_agents(orchestrator) {
            tracing::warn!("could not update the agents' Responses items: {e}");
        }
        if !orchestrator.assistant_enabled() {
            // Keep the clock rolling so the poll does not fire the moment
            // the assistant comes back on.
            self.last_poll = Instant::now();
            return;
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

    /// File every agent waiting on the user, and drop the items of agents
    /// that stopped waiting or are gone.
    fn check_agents(&mut self, orchestrator: &Orchestrator) -> crate::error::CoreResult<()> {
        let processes = orchestrator.processes_with_subagents();
        // Process ids are per run, so this also clears what an earlier run
        // left; restarts and crashes are handled as they happen (`on_event`).
        orchestrator
            .reconcile_agent_responses(&processes.iter().map(|p| p.id).collect::<Vec<_>>())?;
        // What each waiting agent waits for, when its provider says, and
        // when that wait began (the later of the two sources).
        let now = Instant::now();
        let mut waiting: HashMap<ProcessId, (Option<String>, Instant)> = orchestrator
            .agents_awaiting_input()
            .into_iter()
            .map(|(id, since)| (id, (None, since)))
            .collect();
        for (id, activity) in orchestrator.output_activity() {
            if activity.state == Some(ReportedState::Waiting) {
                let began = now
                    .checked_sub(Duration::from_millis(activity.state_ms.unwrap_or(0)))
                    .unwrap_or(now);
                let began = waiting.get(&id).map_or(began, |(_, ui)| began.max(*ui));
                waiting.insert(id, (activity.waiting_for, began));
            }
        }
        let ended: Vec<ProcessId> = self
            .asking
            .keys()
            .filter(|id| !waiting.contains_key(id))
            .copied()
            .collect();
        for id in ended {
            self.asking.remove(&id);
            // Only the question: the agent may have filed a review since.
            orchestrator.remove_agent_response_for(id, ResponseReason::Question)?;
        }
        let active = active(orchestrator);
        for process in processes {
            let Some((asks, began)) = waiting.remove(&process.id) else {
                continue;
            };
            if !matches!(process.kind, ProcessKind::Agent { .. })
                || !matches!(process.status, ProcessStatus::Running { .. })
            {
                continue;
            }
            // A sub-agent waiting on its own question (ask_assistant) is quiet
            // on purpose: the assistant already has the question.
            if process
                .subagent
                .as_ref()
                .is_some_and(|s| s.phase == crate::subagent::SubagentPhase::Waiting)
            {
                continue;
            }
            let asking = self.asking.entry(process.id).or_insert_with(Asking::new);
            // Typing into the agent answers it (`write_stdin` dismisses the
            // item). A wait that began after the typing is a new question (a
            // second prompt between two ticks); one from before is the
            // answered prompt still on screen, so its item stays dismissed.
            if orchestrator
                .last_user_input(process.id)
                .is_some_and(|typed| typed > asking.at && began > typed)
            {
                *asking = Asking::new();
            }
            if process.viewing {
                asking.seen = true;
                orchestrator.remove_agent_response(process.id)?;
                continue;
            }
            if asking.seen || !active {
                continue;
            }
            let text = match asks {
                Some(what) => format!("Waiting for {what}."),
                None => "Waiting for your input.".to_string(),
            };
            orchestrator.file_agent_response(
                &process,
                ResponseKind::NeedsYou,
                ResponseReason::Question,
                format!("question:{}", asking.since.timestamp_millis()),
                text,
            )?;
        }
        Ok(())
    }
}

fn exit_code(code: Option<i32>) -> String {
    match code {
        Some(code) => format!("exit code {code}"),
        None => "no exit code".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An agent that stops waiting loses its question, not a review it
    /// filed meanwhile (`complete_todo` while the question was up).
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(windows, ignore = "runs `cat` in a PTY")]
    async fn a_question_that_ends_leaves_a_review_alone() {
        let dir = tempfile::tempdir().unwrap();
        let orch = Arc::new(Orchestrator::new());
        let project = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        let id = orch
            .add_process(
                project,
                crate::process::ProcessSpec {
                    name: "fix-login".to_string(),
                    command: "cat".to_string(),
                    cwd: dir.path().to_path_buf(),
                    env: Vec::new(),
                    kind: ProcessKind::Agent {
                        adapter: "claude-code".to_string(),
                    },
                    restart_policy: crate::process::RestartPolicy::Never,
                    color: None,
                },
            )
            .await
            .unwrap();
        orch.start_process(id).await.unwrap();
        let info = orch.process_info(id).unwrap();
        let mut watcher = Watcher {
            weak: Arc::downgrade(&orch),
            config: ProactivityConfig::default(),
            asking: HashMap::new(),
            last_poll: Instant::now(),
        };

        // It was asking; now it is not, and it filed a review.
        watcher.asking.insert(id, Asking::new());
        orch.file_review_response(&info, "Login fixed.".to_string())
            .unwrap();
        watcher.check_agents(&orch).unwrap();
        let items = orch.assistant_responses();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].reason, ResponseReason::Review);

        // A question still goes when the wait ends.
        orch.file_agent_response(
            &info,
            ResponseKind::NeedsYou,
            ResponseReason::Question,
            "question:1".to_string(),
            "Waiting for your input.".to_string(),
        )
        .unwrap();
        watcher.asking.insert(id, Asking::new());
        watcher.check_agents(&orch).unwrap();
        assert!(orch.assistant_responses().is_empty());
        orch.shutdown().await;
    }

    /// Typing into an agent or opening it takes its question off the list at
    /// once. The next tick leaves it off while the answered prompt is still
    /// up; a prompt that comes after the typing is a new question.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(windows, ignore = "runs `stty` and `sleep` in a PTY")]
    async fn answering_or_opening_an_agent_clears_its_question_until_a_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let orch = Arc::new(Orchestrator::new());
        orch.set_assistant_enabled(true).await.unwrap();
        let project = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        let id = orch
            .add_process(
                project,
                crate::process::ProcessSpec {
                    name: "fix-login".to_string(),
                    // No echo and nothing read: typing leaves the prompt "up".
                    command: "stty -echo; sleep 30".to_string(),
                    cwd: dir.path().to_path_buf(),
                    env: Vec::new(),
                    kind: ProcessKind::Agent {
                        adapter: "claude-code".to_string(),
                    },
                    restart_policy: crate::process::RestartPolicy::Never,
                    color: None,
                },
            )
            .await
            .unwrap();
        orch.start_process(id).await.unwrap();
        let mut watcher = Watcher {
            weak: Arc::downgrade(&orch),
            config: ProactivityConfig::default(),
            asking: HashMap::new(),
            last_poll: Instant::now(),
        };
        let pause = || tokio::time::sleep(Duration::from_millis(20));
        // Let the shell settle so no late output reads as an answer.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let questions = || {
            orch.assistant_responses()
                .into_iter()
                .filter(|i| i.reason == ResponseReason::Question)
                .count()
        };

        orch.report_agent_waiting(id);
        watcher.check_agents(&orch).unwrap();
        assert_eq!(questions(), 1);

        // Typed an answer: gone at once, and not back while the prompt is up.
        pause().await;
        orch.write_stdin(id, b"y").await.unwrap();
        assert_eq!(questions(), 0);
        watcher.check_agents(&orch).unwrap();
        assert_eq!(questions(), 0);

        // A prompt after the typing is a new question.
        pause().await;
        orch.report_agent_waiting(id);
        watcher.check_agents(&orch).unwrap();
        assert_eq!(questions(), 1);

        // Opened: gone at once, and not back after leaving it.
        orch.set_viewing(Some(id));
        assert_eq!(questions(), 0);
        orch.set_viewing(None);
        watcher.check_agents(&orch).unwrap();
        assert_eq!(questions(), 0);
        orch.shutdown().await;
    }

    #[test]
    fn an_exit_without_a_code_is_still_reported() {
        assert_eq!(exit_code(Some(0)), "exit code 0");
        assert_eq!(exit_code(None), "no exit code");
    }
}
