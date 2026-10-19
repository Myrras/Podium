//! The personal assistant's sub-agents: work it hands off so it can answer
//! the user straight away instead of doing the work in its own turn.
//!
//! A sub-agent is an ordinary Podium agent process (PTY, Claude Code adapter)
//! that belongs to the assistant rather than to a project: it runs in the
//! assistant's own scope, a Podium-internal project rooted at
//! `{app_data_dir}/assistant-workspace` that no listing shows and no
//! user-facing project call accepts (see `Orchestrator::set_assistant_workspace`).
//! This module holds what is specific to them — the kind, the task, the
//! lifecycle phase and the cap on how many run at once — while the
//! orchestrator does the spawning, stopping and nudging.
//!
//! Lifecycle: a sub-agent is admitted as `running` when one of the
//! [`SubagentConfig::max_running`] slots is free, else `queued`, and a queued
//! one starts (oldest first) as soon as a slot frees. It ends `done` when it
//! reports its result (`report_to_assistant`), `failed` when it exits without
//! one, or `stopped` when it was stopped (or cancelled while queued). Every
//! ended phase frees its slot; the row stays visible for
//! [`SubagentConfig::linger`] and is then removed together with its process.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::ProcessId;
use crate::process::ProcessStatus;

/// How many sub-agents may run at once; more wait their turn.
pub const MAX_RUNNING_SUBAGENTS: usize = 4;

/// The tools a lookup sub-agent may never run — the assistant's own list:
/// no shell, no file edits.
pub const LOOKUP_DISALLOWED_TOOLS: &str = "Bash,Edit,Write,NotebookEdit";

/// Longest reported result kept on a sub-agent's row (and handed on to the
/// assistant); the sub-agent is asked for two lines, so this only bounds a
/// runaway report.
const MAX_RESULT_CHARS: usize = 600;

/// Longest slice of the task quoted back in a nudge, so a result that comes
/// back into a fresh conversation still says what it was for.
const MAX_TASK_IN_NUDGE: usize = 80;

/// What a sub-agent may do, picked by the assistant per task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SubagentKind {
    /// Reads and looks things up (Slack, Teams, tickets, MCP reads, a summary
    /// across projects): no shell, no file edits.
    Lookup,
    /// Shell, files and git, for work that belongs to no single project.
    Full,
}

impl SubagentKind {
    /// The wire spelling, also used in nudges.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lookup => "lookup",
            Self::Full => "full",
        }
    }
}

/// Where a sub-agent is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubagentPhase {
    /// Waiting for a free slot.
    Queued,
    Running,
    /// Reported its result.
    Done,
    /// Exited without a result, or could not start.
    Failed,
    /// Stopped by the user or the assistant, or cancelled while queued.
    Stopped,
}

impl SubagentPhase {
    /// Whether the sub-agent is over: its slot is free and its row will drop
    /// off after [`SubagentConfig::linger`].
    pub fn is_ended(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Stopped)
    }
}

/// Timings and limits of the sub-agent machinery. Injectable, so tests run
/// in milliseconds and can point the launch at a fake adapter.
#[derive(Debug, Clone)]
pub struct SubagentConfig {
    /// How many run at once.
    pub max_running: usize,
    /// How long an ended sub-agent stays listed before its row and process
    /// are removed.
    pub linger: Duration,
    /// How long a sub-agent that reported its result keeps running before
    /// Podium stops it — time for the CLI to finish the tool call cleanly.
    pub report_grace: Duration,
    /// How often ended rows are swept (and missed exits reconciled).
    pub tick: Duration,
    /// How long after a start Podium watches for Claude Code's folder-trust
    /// prompt (see [`shows_trust_prompt`]).
    pub trust_window: Duration,
    /// The adapter sub-agents launch with.
    pub adapter_id: String,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            max_running: MAX_RUNNING_SUBAGENTS,
            linger: Duration::from_secs(5 * 60),
            report_grace: Duration::from_secs(5),
            tick: Duration::from_secs(10),
            trust_window: Duration::from_secs(30),
            adapter_id: "claude-code".to_string(),
        }
    }
}

/// UI-facing snapshot of one sub-agent: the panel's row, `list_subagents`'
/// answer and the `subagentsChanged` event's payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentInfo {
    /// Its process id: what `process_attach` (the terminal), the output tail
    /// and a stop take.
    pub id: ProcessId,
    pub name: String,
    pub kind: SubagentKind,
    /// The task it was given, as the assistant wrote it.
    pub task: String,
    pub phase: SubagentPhase,
    /// Its process's own status (a queued one has not started yet).
    pub status: ProcessStatus,
    pub created_at: DateTime<Utc>,
    /// When it got a slot; `None` while queued.
    pub started_at: Option<DateTime<Utc>>,
    /// When it ended (reported, exited or was stopped).
    pub ended_at: Option<DateTime<Utc>>,
    /// What it reported, once `done`.
    pub result: Option<String>,
    /// Why it failed, once `failed`.
    pub error: Option<String>,
}

/// What a sub-agent process carries on its [`crate::ProcessInfo`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentMeta {
    pub kind: SubagentKind,
    pub task: String,
    pub phase: SubagentPhase,
    pub ended_at: Option<DateTime<Utc>>,
}

/// What `spawn_subagent` answers: the new process and whether it started
/// (`running`) or waits for a slot (`queued`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentSpawned {
    pub process_id: ProcessId,
    pub state: SubagentPhase,
}

/// One sub-agent's bookkeeping.
#[derive(Debug, Clone)]
pub(crate) struct SubagentRecord {
    pub kind: SubagentKind,
    pub task: String,
    pub phase: SubagentPhase,
    /// Admission order: the queue is first come, first served.
    seq: u64,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    /// The same moment on the monotonic clock, for the linger timer.
    ended: Option<Instant>,
    pub result: Option<String>,
    pub error: Option<String>,
    /// The stop came from the user (the panel's stop), so the assistant is
    /// told. A stop the assistant asked for itself (`stop_process`), or a new
    /// conversation that cleared them, is not news to anyone.
    pub stopped_by_user: bool,
}

/// Every sub-agent of the run, keyed by process id. Pure bookkeeping — no
/// processes are touched here, so the cap and the phase rules are unit
/// testable on their own.
#[derive(Debug, Default)]
pub(crate) struct SubagentBook {
    records: HashMap<ProcessId, SubagentRecord>,
    admitted: u64,
}

impl SubagentBook {
    pub fn contains(&self, id: &ProcessId) -> bool {
        self.records.contains_key(id)
    }

    pub fn get(&self, id: &ProcessId) -> Option<&SubagentRecord> {
        self.records.get(id)
    }

    pub fn get_mut(&mut self, id: &ProcessId) -> Option<&mut SubagentRecord> {
        self.records.get_mut(id)
    }

    /// How many hold a slot.
    pub fn running(&self) -> usize {
        self.count(SubagentPhase::Running)
    }

    /// How many wait for one.
    pub fn queued(&self) -> usize {
        self.count(SubagentPhase::Queued)
    }

    fn count(&self, phase: SubagentPhase) -> usize {
        self.records.values().filter(|r| r.phase == phase).count()
    }

    /// Add sub-agent `id`: `Running` when fewer than `max_running` hold a
    /// slot — and nothing is already waiting, so a newcomer never jumps the
    /// queue — else `Queued`.
    pub fn admit(
        &mut self,
        id: ProcessId,
        kind: SubagentKind,
        task: &str,
        max_running: usize,
        now: DateTime<Utc>,
    ) -> SubagentPhase {
        let phase = if self.running() < max_running && self.queued() == 0 {
            SubagentPhase::Running
        } else {
            SubagentPhase::Queued
        };
        self.admitted += 1;
        self.records.insert(
            id,
            SubagentRecord {
                kind,
                task: task.to_string(),
                phase,
                seq: self.admitted,
                created_at: now,
                started_at: (phase == SubagentPhase::Running).then_some(now),
                ended_at: None,
                ended: None,
                result: None,
                error: None,
                stopped_by_user: false,
            },
        );
        phase
    }

    /// Hand the free slots to the oldest queued sub-agents, marking them
    /// `Running`; returns them oldest first, for the caller to start.
    pub fn promote(&mut self, max_running: usize, now: DateTime<Utc>) -> Vec<ProcessId> {
        let free = max_running.saturating_sub(self.running());
        let mut queued: Vec<(u64, ProcessId)> = self
            .records
            .iter()
            .filter(|(_, r)| r.phase == SubagentPhase::Queued)
            .map(|(id, r)| (r.seq, *id))
            .collect();
        queued.sort_unstable_by_key(|(seq, _)| *seq);
        let promoted: Vec<ProcessId> = queued.into_iter().take(free).map(|(_, id)| id).collect();
        for id in &promoted {
            if let Some(r) = self.records.get_mut(id) {
                r.phase = SubagentPhase::Running;
                r.started_at = Some(now);
            }
        }
        promoted
    }

    /// Move `id` into an ended `phase`. An ended sub-agent stays as it ended
    /// — a stop after its report is still `done` — so this answers whether
    /// anything changed.
    pub fn finish(
        &mut self,
        id: &ProcessId,
        phase: SubagentPhase,
        outcome: Option<String>,
        now: DateTime<Utc>,
        at: Instant,
    ) -> bool {
        debug_assert!(phase.is_ended());
        let Some(r) = self.records.get_mut(id) else {
            return false;
        };
        if r.phase.is_ended() {
            return false;
        }
        r.phase = phase;
        r.ended_at = Some(now);
        r.ended = Some(at);
        match phase {
            SubagentPhase::Done => {
                r.result = outcome.map(|o| crate::assistant::truncate(&o, MAX_RESULT_CHARS))
            }
            SubagentPhase::Failed => r.error = outcome,
            _ => {}
        }
        true
    }

    /// Ended sub-agents whose row has been shown for `linger` already.
    pub fn expired(&self, linger: Duration, now: Instant) -> Vec<ProcessId> {
        self.records
            .iter()
            .filter(|(_, r)| r.ended.is_some_and(|at| now.duration_since(at) >= linger))
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn remove(&mut self, id: &ProcessId) -> Option<SubagentRecord> {
        self.records.remove(id)
    }

    /// Every id, oldest first.
    pub fn ids(&self) -> Vec<ProcessId> {
        let mut ids: Vec<(u64, ProcessId)> =
            self.records.iter().map(|(id, r)| (r.seq, *id)).collect();
        ids.sort_unstable_by_key(|(seq, _)| *seq);
        ids.into_iter().map(|(_, id)| id).collect()
    }
}

impl SubagentRecord {
    pub fn meta(&self) -> SubagentMeta {
        SubagentMeta {
            kind: self.kind,
            task: self.task.clone(),
            phase: self.phase,
            ended_at: self.ended_at,
        }
    }

    pub fn info(&self, id: ProcessId, name: String, status: ProcessStatus) -> SubagentInfo {
        SubagentInfo {
            id,
            name,
            kind: self.kind,
            task: self.task.clone(),
            phase: self.phase,
            status,
            created_at: self.created_at,
            started_at: self.started_at,
            ended_at: self.ended_at,
            result: self.result.clone(),
            error: self.error.clone(),
        }
    }
}

/// Extra CLI args a sub-agent of `kind` launches with, on top of the user's
/// default args (Claude Code flags: sub-agents always run on it).
///
/// A lookup mirrors the assistant itself: the same tools refused, and no
/// permission prompts — it runs in the background, where a prompt for an MCP
/// read (Slack, a ticket) would only ever block it. That leaves it nothing
/// the assistant could not already do on its own. A full sub-agent keeps the
/// normal prompts: one waiting on a shell command is reported like a stuck
/// agent, and the user answers it in its terminal.
pub fn launch_args(kind: SubagentKind) -> Vec<String> {
    match kind {
        SubagentKind::Lookup => vec![
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
            "--disallowedTools".to_string(),
            LOOKUP_DISALLOWED_TOOLS.to_string(),
        ],
        SubagentKind::Full => Vec::new(),
    }
}

/// Whether `screen` (a sub-agent's ANSI-stripped output) shows Claude Code's
/// folder-trust prompt. The assistant workspace is a folder Podium made for
/// itself, so its first sub-agent would otherwise sit on that question with
/// nobody looking; the answer persists in the CLI's own config, so this only
/// ever happens once per machine.
pub fn shows_trust_prompt(screen: &str) -> bool {
    let screen = screen.to_lowercase();
    screen.contains("trust this folder") || screen.contains("do you trust the files")
}

/// `"name" (process <id>, <kind>) for "<task>"` — how a nudge names one.
fn who(name: &str, id: ProcessId, kind: SubagentKind, task: &str) -> String {
    format!(
        "\"{name}\" (process {id}, {}) for \"{}\"",
        kind.as_str(),
        crate::assistant::truncate(task.trim(), MAX_TASK_IN_NUDGE)
    )
}

/// The nudge for a sub-agent that reported its result.
pub fn finished_nudge(
    name: &str,
    id: ProcessId,
    kind: SubagentKind,
    task: &str,
    result: &str,
) -> String {
    format!(
        "[Podium] Your sub-agent {} finished. Its result: {} Tell the user in at most two \
         lines. If it says follow-up work is needed, add a to-do for it in the relevant \
         project (add_todo); if no project fits or you cannot tell which, ask the user.",
        who(name, id, kind, task),
        result.trim()
    )
}

/// The nudge for a sub-agent that ended without a result: one line, the
/// reason, nothing to summarise.
pub fn failed_nudge(
    name: &str,
    id: ProcessId,
    kind: SubagentKind,
    task: &str,
    reason: &str,
) -> String {
    format!(
        "[Podium] Your sub-agent {} failed: {reason}. Tell the user in one line, with the \
         reason.",
        who(name, id, kind, task)
    )
}

/// The nudge for a sub-agent the user stopped from the panel.
pub fn stopped_by_user_nudge(name: &str, id: ProcessId, kind: SubagentKind, task: &str) -> String {
    format!(
        "[Podium] Your sub-agent {} was stopped by the user, no summary needed.",
        who(name, id, kind, task)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book_with(n: usize) -> (SubagentBook, Vec<(ProcessId, SubagentPhase)>) {
        let mut book = SubagentBook::default();
        let admitted = (0..n)
            .map(|i| {
                let id = ProcessId::new();
                let phase = book.admit(
                    id,
                    SubagentKind::Lookup,
                    &format!("task {i}"),
                    MAX_RUNNING_SUBAGENTS,
                    Utc::now(),
                );
                (id, phase)
            })
            .collect();
        (book, admitted)
    }

    #[test]
    fn four_run_at_once_and_the_fifth_waits() {
        let (book, admitted) = book_with(5);
        let phases: Vec<SubagentPhase> = admitted.iter().map(|(_, p)| *p).collect();
        assert_eq!(
            phases,
            vec![
                SubagentPhase::Running,
                SubagentPhase::Running,
                SubagentPhase::Running,
                SubagentPhase::Running,
                SubagentPhase::Queued,
            ]
        );
        assert_eq!(book.running(), 4);
        assert_eq!(book.queued(), 1);
        // A queued one has not started.
        assert!(book.get(&admitted[4].0).unwrap().started_at.is_none());
    }

    #[test]
    fn an_ended_one_frees_its_slot_for_the_oldest_queued() {
        let (mut book, admitted) = book_with(6);
        // Nothing to promote while all slots are taken.
        assert!(book.promote(MAX_RUNNING_SUBAGENTS, Utc::now()).is_empty());

        assert!(book.finish(
            &admitted[1].0,
            SubagentPhase::Done,
            Some("found it".to_string()),
            Utc::now(),
            Instant::now(),
        ));
        let promoted = book.promote(MAX_RUNNING_SUBAGENTS, Utc::now());
        assert_eq!(promoted, vec![admitted[4].0], "first come, first served");
        let fifth = book.get(&admitted[4].0).unwrap();
        assert_eq!(fifth.phase, SubagentPhase::Running);
        assert!(fifth.started_at.is_some());
        assert_eq!(book.queued(), 1);
        assert_eq!(book.running(), 4);
    }

    #[test]
    fn a_newcomer_never_jumps_the_queue() {
        let (mut book, admitted) = book_with(5);
        book.finish(
            &admitted[0].0,
            SubagentPhase::Failed,
            Some("it crashed".to_string()),
            Utc::now(),
            Instant::now(),
        );
        // A slot is free, but the fifth was here first.
        let late = ProcessId::new();
        assert_eq!(
            book.admit(late, SubagentKind::Full, "late", 4, Utc::now()),
            SubagentPhase::Queued
        );
        assert_eq!(book.promote(4, Utc::now()), vec![admitted[4].0]);
    }

    #[test]
    fn an_ended_sub_agent_stays_as_it_ended() {
        let (mut book, admitted) = book_with(1);
        let id = admitted[0].0;
        assert!(book.finish(
            &id,
            SubagentPhase::Done,
            Some("two lines".to_string()),
            Utc::now(),
            Instant::now(),
        ));
        // The stop that follows its report changes nothing.
        assert!(!book.finish(
            &id,
            SubagentPhase::Stopped,
            None,
            Utc::now(),
            Instant::now()
        ));
        let r = book.get(&id).unwrap();
        assert_eq!(r.phase, SubagentPhase::Done);
        assert_eq!(r.result.as_deref(), Some("two lines"));
        assert!(r.ended_at.is_some());
    }

    #[test]
    fn a_failure_keeps_its_reason_and_a_runaway_result_is_clipped() {
        let (mut book, admitted) = book_with(2);
        book.finish(
            &admitted[0].0,
            SubagentPhase::Failed,
            Some("it crashed (exit code 1)".to_string()),
            Utc::now(),
            Instant::now(),
        );
        assert_eq!(
            book.get(&admitted[0].0).unwrap().error.as_deref(),
            Some("it crashed (exit code 1)")
        );
        book.finish(
            &admitted[1].0,
            SubagentPhase::Done,
            Some("x".repeat(MAX_RESULT_CHARS * 2)),
            Utc::now(),
            Instant::now(),
        );
        let result = book.get(&admitted[1].0).unwrap().result.clone().unwrap();
        assert!(result.chars().count() <= MAX_RESULT_CHARS + 1, "{result}");
    }

    #[test]
    fn a_queued_one_can_be_cancelled_without_ever_running() {
        let (mut book, admitted) = book_with(5);
        let queued = admitted[4].0;
        assert!(book.finish(
            &queued,
            SubagentPhase::Stopped,
            None,
            Utc::now(),
            Instant::now()
        ));
        assert_eq!(book.queued(), 0);
        assert_eq!(book.running(), 4);
        assert!(book.get(&queued).unwrap().started_at.is_none());
    }

    #[test]
    fn ended_rows_expire_after_the_linger_and_live_ones_never_do() {
        let (mut book, admitted) = book_with(2);
        let ended_at = Instant::now();
        book.finish(
            &admitted[0].0,
            SubagentPhase::Done,
            None,
            Utc::now(),
            ended_at,
        );
        let linger = Duration::from_secs(300);
        assert!(book.expired(linger, ended_at).is_empty(), "too soon");
        assert!(book
            .expired(linger, ended_at + Duration::from_secs(299))
            .is_empty());
        assert_eq!(
            book.expired(linger, ended_at + linger),
            vec![admitted[0].0],
            "only the ended one, once its linger is up"
        );
        book.remove(&admitted[0].0);
        assert_eq!(book.ids(), vec![admitted[1].0]);
    }

    #[test]
    fn ids_come_oldest_first() {
        let (book, admitted) = book_with(6);
        let ids: Vec<ProcessId> = admitted.iter().map(|(id, _)| *id).collect();
        assert_eq!(book.ids(), ids);
    }

    #[test]
    fn a_lookup_mirrors_the_assistant_and_a_full_one_keeps_its_prompts() {
        assert_eq!(
            launch_args(SubagentKind::Lookup),
            vec![
                "--permission-mode",
                "bypassPermissions",
                "--disallowedTools",
                "Bash,Edit,Write,NotebookEdit",
            ]
        );
        assert!(launch_args(SubagentKind::Full).is_empty());
    }

    #[test]
    fn kinds_and_phases_are_camel_case_on_the_wire() {
        assert_eq!(
            serde_json::to_value(SubagentKind::Lookup).unwrap(),
            "lookup"
        );
        assert_eq!(serde_json::to_value(SubagentKind::Full).unwrap(), "full");
        for (phase, wire) in [
            (SubagentPhase::Queued, "queued"),
            (SubagentPhase::Running, "running"),
            (SubagentPhase::Done, "done"),
            (SubagentPhase::Failed, "failed"),
            (SubagentPhase::Stopped, "stopped"),
        ] {
            assert_eq!(serde_json::to_value(phase).unwrap(), wire);
        }
        let kind: SubagentKind = serde_json::from_str("\"full\"").unwrap();
        assert_eq!(kind, SubagentKind::Full);
    }

    #[test]
    fn the_trust_prompt_is_recognised_and_ordinary_output_is_not() {
        assert!(shows_trust_prompt(
            "Quick safety check: Is this a project you created or one you trust?\n❯ 1. Yes, I trust this folder\n  2. No, exit"
        ));
        assert!(shows_trust_prompt("Do you trust the files in this folder?"));
        assert!(!shows_trust_prompt(
            "Searching Slack for the release thread…"
        ));
    }

    #[test]
    fn a_finished_nudge_asks_for_two_lines_and_a_to_do_when_follow_up_is_needed() {
        let id = ProcessId::new();
        let text = finished_nudge(
            "release-thread",
            id,
            SubagentKind::Lookup,
            "Find the release thread in Slack",
            "Found it: QA signs off on Friday. Follow-up: update the changelog in Webshop.",
        );
        assert!(text.starts_with("[Podium] Your sub-agent \"release-thread\""));
        assert!(text.contains(&id.to_string()), "names the process: {text}");
        assert!(text.contains("lookup"));
        assert!(text.contains("Find the release thread in Slack"));
        assert!(text.contains("QA signs off on Friday"));
        assert!(text.contains("at most two lines"));
        assert!(text.contains("add_todo"));
    }

    #[test]
    fn failure_and_user_stop_nudges_are_one_liners() {
        let id = ProcessId::new();
        let failed = failed_nudge(
            "cross-repo-check",
            id,
            SubagentKind::Full,
            "Check every repo for the old logger",
            "it crashed (exit code 2)",
        );
        assert!(
            failed.contains("failed: it crashed (exit code 2)"),
            "{failed}"
        );
        assert!(failed.contains("one line"), "{failed}");
        assert!(failed.contains(&id.to_string()));

        let stopped = stopped_by_user_nudge("cross-repo-check", id, SubagentKind::Full, "x");
        assert!(stopped.contains("stopped by the user, no summary needed"));
        assert!(stopped.contains(&id.to_string()));
    }

    #[test]
    fn a_long_task_is_clipped_in_the_nudge() {
        let text = failed_nudge(
            "n",
            ProcessId::new(),
            SubagentKind::Full,
            &"word ".repeat(100),
            "r",
        );
        assert!(text.len() < 300, "{text}");
    }
}
