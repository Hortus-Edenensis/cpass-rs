use std::collections::VecDeque;
use std::io::{self, IsTerminal, Write};
use std::sync::Mutex;

use cpass_core::event::{RunEvent, RunEventSink};
use cpass_core::{CourseRunExecutionState, CourseRunQueueEntryState};

const MAX_WARNINGS: usize = 4;
const MAX_RECENT_EVENTS: usize = 8;
const MAX_QUEUE_LINES: usize = 12;

pub struct RunTui {
    inner: Mutex<RunTuiInner>,
}

struct RunTuiInner {
    state: RunTuiState,
    interactive: bool,
    active: bool,
}

impl RunTui {
    pub fn new() -> io::Result<Self> {
        let interactive = io::stdout().is_terminal()
            && std::env::var("TERM")
                .map(|term| term != "dumb")
                .unwrap_or(true);
        let tui = Self {
            inner: Mutex::new(RunTuiInner {
                state: RunTuiState::default(),
                interactive,
                active: false,
            }),
        };
        if interactive {
            tui.enter()?;
        }
        Ok(tui)
    }

    pub fn finish(&self) -> io::Result<()> {
        let (interactive, active, snapshot) = {
            let mut inner = self.inner.lock().expect("run tui poisoned");
            let active = inner.active;
            inner.active = false;
            (
                inner.interactive,
                active,
                inner.state.render_text_snapshot(),
            )
        };

        let mut stdout = io::stdout().lock();
        if interactive && active {
            write!(stdout, "\x1b[?25h\x1b[?1049l")?;
        }
        writeln!(stdout, "{snapshot}")?;
        stdout.flush()
    }

    fn enter(&self) -> io::Result<()> {
        {
            let mut inner = self.inner.lock().expect("run tui poisoned");
            inner.active = true;
        }
        let mut stdout = io::stdout().lock();
        write!(stdout, "\x1b[?1049h\x1b[?25l")?;
        stdout.flush()
    }

    fn redraw(&self) -> io::Result<()> {
        let snapshot = {
            let inner = self.inner.lock().expect("run tui poisoned");
            if !inner.interactive {
                return Ok(());
            }
            inner.state.render_text_snapshot()
        };
        let mut stdout = io::stdout().lock();
        write!(stdout, "\x1b[2J\x1b[H{snapshot}")?;
        stdout.flush()
    }
}

impl RunEventSink for RunTui {
    fn emit(&self, event: RunEvent) {
        {
            let mut inner = self.inner.lock().expect("run tui poisoned");
            inner.state.apply_event(&event);
        }
        let _ = self.redraw();
    }
}

impl Drop for RunTui {
    fn drop(&mut self) {
        let cleanup = {
            let mut inner = self.inner.lock().expect("run tui poisoned");
            if inner.interactive && inner.active {
                inner.active = false;
                true
            } else {
                false
            }
        };
        if cleanup {
            let mut stdout = io::stdout().lock();
            let _ = write!(stdout, "\x1b[?25h\x1b[?1049l");
            let _ = stdout.flush();
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct RunTuiState {
    requested_course_id: Option<u64>,
    requested_course_index: Option<usize>,
    resolved_course_id: Option<u64>,
    planning_started: bool,
    planning_finished: bool,
    chapter_count: usize,
    planned_task_points: usize,
    execution_started: bool,
    execution_state: Option<CourseRunExecutionState>,
    total_entries: usize,
    completed_entries: usize,
    blocked_entries: usize,
    queue_states: Vec<CourseRunQueueEntryState>,
    warnings: VecDeque<String>,
    recent_events: VecDeque<String>,
}

impl RunTuiState {
    fn apply_event(&mut self, event: &RunEvent) {
        self.push_recent_event(describe_event(event));
        match event {
            RunEvent::CourseRunPlanningStarted {
                course_id,
                course_index,
            } => {
                self.planning_started = true;
                self.requested_course_id = *course_id;
                self.requested_course_index = *course_index;
            }
            RunEvent::CourseRunPlanningFinished {
                course_id,
                chapters,
                task_points,
            } => {
                self.planning_finished = true;
                self.resolved_course_id = Some(*course_id);
                self.chapter_count = *chapters;
                self.planned_task_points = *task_points;
            }
            RunEvent::CourseRunExecutionStarted {
                course_id,
                total_entries,
            } => {
                self.execution_started = true;
                self.resolved_course_id = Some(*course_id);
                self.execution_state = Some(CourseRunExecutionState::Running);
                self.total_entries = *total_entries;
                self.completed_entries = 0;
                self.blocked_entries = 0;
                self.queue_states = vec![CourseRunQueueEntryState::Pending; *total_entries];
            }
            RunEvent::CourseRunQueueEntryStateChanged {
                queue_index, state, ..
            } => {
                self.ensure_queue_len(*queue_index + 1);
                self.queue_states[*queue_index] = *state;
                self.completed_entries = self
                    .queue_states
                    .iter()
                    .filter(|entry| **entry == CourseRunQueueEntryState::Completed)
                    .count();
                self.blocked_entries = self
                    .queue_states
                    .iter()
                    .filter(|entry| **entry == CourseRunQueueEntryState::Blocked)
                    .count();
            }
            RunEvent::CourseRunExecutionFinished {
                course_id,
                state,
                total_entries,
                completed_entries,
                blocked_entries,
            } => {
                self.resolved_course_id = Some(*course_id);
                self.execution_state = Some(*state);
                self.total_entries = *total_entries;
                self.completed_entries = *completed_entries;
                self.blocked_entries = *blocked_entries;
                self.ensure_queue_len(*total_entries);
            }
            RunEvent::Warning { message } => self.push_warning(message.clone()),
            _ => {}
        }
    }

    fn ensure_queue_len(&mut self, len: usize) {
        if self.queue_states.len() < len {
            self.queue_states
                .resize(len, CourseRunQueueEntryState::Pending);
        }
    }

    fn push_warning(&mut self, warning: String) {
        push_capped(&mut self.warnings, warning, MAX_WARNINGS);
    }

    fn push_recent_event(&mut self, event: String) {
        push_capped(&mut self.recent_events, event, MAX_RECENT_EVENTS);
    }

    fn render_text_snapshot(&self) -> String {
        let mut lines = vec![
            "cpass run TUI".to_owned(),
            "==============".to_owned(),
            format!("Planning: {}", self.planning_line()),
            format!("Execution: {}", self.execution_line()),
        ];

        if self.total_entries > 0 {
            lines.push(String::new());
            lines.push(format!(
                "Queue: {} completed, {} blocked, {} pending",
                self.completed_entries,
                self.blocked_entries,
                self.pending_entries(),
            ));
            for (index, state) in self.queue_states.iter().enumerate().take(MAX_QUEUE_LINES) {
                lines.push(format!("  [{index}] {}", state.as_str()));
            }
            if self.queue_states.len() > MAX_QUEUE_LINES {
                lines.push(format!(
                    "  ... {} more entr{}",
                    self.queue_states.len() - MAX_QUEUE_LINES,
                    if self.queue_states.len() - MAX_QUEUE_LINES == 1 {
                        "y"
                    } else {
                        "ies"
                    }
                ));
            }
        }

        if !self.warnings.is_empty() {
            lines.push(String::new());
            lines.push("Warnings:".to_owned());
            for warning in &self.warnings {
                lines.push(format!("  - {warning}"));
            }
        }

        if !self.recent_events.is_empty() {
            lines.push(String::new());
            lines.push("Recent events:".to_owned());
            for event in &self.recent_events {
                lines.push(format!("  - {event}"));
            }
        }

        lines.join("\n")
    }

    fn pending_entries(&self) -> usize {
        self.total_entries
            .saturating_sub(self.completed_entries + self.blocked_entries + self.running_entries())
    }

    fn running_entries(&self) -> usize {
        self.queue_states
            .iter()
            .filter(|state| **state == CourseRunQueueEntryState::Running)
            .count()
    }

    fn planning_line(&self) -> String {
        if self.planning_finished {
            return format!(
                "finished for course {} ({} chapters, {} task points)",
                self.resolved_course_id
                    .map(|course_id| course_id.to_string())
                    .unwrap_or_else(|| "<unknown>".to_owned()),
                self.chapter_count,
                self.planned_task_points
            );
        }
        if self.planning_started {
            return format!("running for {}", self.request_target());
        }
        "idle".to_owned()
    }

    fn execution_line(&self) -> String {
        match self.execution_state {
            Some(state) => format!(
                "{} for course {} ({} total entries)",
                state.as_str(),
                self.resolved_course_id
                    .map(|course_id| course_id.to_string())
                    .unwrap_or_else(|| "<unknown>".to_owned()),
                self.total_entries
            ),
            None if self.execution_started => format!(
                "running for course {} ({} total entries)",
                self.resolved_course_id
                    .map(|course_id| course_id.to_string())
                    .unwrap_or_else(|| "<unknown>".to_owned()),
                self.total_entries
            ),
            None if self.planning_started => "waiting for planner".to_owned(),
            None => "idle".to_owned(),
        }
    }

    fn request_target(&self) -> String {
        if let Some(course_id) = self.requested_course_id {
            return format!("course_id={course_id}");
        }
        if let Some(course_index) = self.requested_course_index {
            return format!("course_index={course_index}");
        }
        "requested target".to_owned()
    }
}

fn push_capped(queue: &mut VecDeque<String>, value: String, cap: usize) {
    if queue.len() == cap {
        queue.pop_front();
    }
    queue.push_back(value);
}

fn describe_event(event: &RunEvent) -> String {
    match event {
        RunEvent::CourseRunPlanningStarted {
            course_id,
            course_index,
        } => format!(
            "planning started ({})",
            if let Some(course_id) = course_id {
                format!("course_id={course_id}")
            } else if let Some(course_index) = course_index {
                format!("course_index={course_index}")
            } else {
                "no target".to_owned()
            }
        ),
        RunEvent::CourseRunPlanningFinished {
            course_id,
            chapters,
            task_points,
        } => format!(
            "planning finished for course {course_id} ({chapters} chapters, {task_points} task points)"
        ),
        RunEvent::CourseRunExecutionStarted {
            course_id,
            total_entries,
        } => format!("execution started for course {course_id} ({total_entries} entries)"),
        RunEvent::CourseRunQueueEntryStateChanged {
            queue_index, state, ..
        } => format!("queue[{queue_index}] -> {}", state.as_str()),
        RunEvent::CourseRunVideoProgressReported {
            queue_index,
            playing_time_secs,
            duration_secs,
            is_passed,
            ..
        } => format!(
            "video queue[{queue_index}] reported {playing_time_secs}/{duration_secs}s (is_passed={is_passed})"
        ),
        RunEvent::CourseRunDocumentProgressReported {
            queue_index,
            success,
            ..
        } => format!("document queue[{queue_index}] reported success={success}"),
        RunEvent::CourseRunChapterWorkSnapshotFetched {
            queue_index,
            snapshot,
            ..
        } => format!(
            "chapter_work queue[{queue_index}] fetched snapshot {} ({} questions)",
            snapshot.work_answer_id,
            snapshot.questions.len()
        ),
        RunEvent::CourseRunChapterWorkCandidateSelectionPrepared {
            queue_index,
            work_answer_id,
            selected_questions,
            total_questions,
            ..
        } => format!(
            "chapter_work queue[{queue_index}] prepared candidate selections {selected_questions}/{total_questions} for snapshot {work_answer_id}"
        ),
        RunEvent::CourseRunExecutionFinished {
            course_id,
            state,
            completed_entries,
            blocked_entries,
            ..
        } => format!(
            "execution finished for course {course_id} ({}, completed={}, blocked={})",
            state.as_str(),
            completed_entries,
            blocked_entries
        ),
        RunEvent::Warning { message } => format!("warning: {message}"),
        other => format!(
            "ignored event: {}",
            serde_json::to_string(other).unwrap_or_default()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::RunTuiState;
    use cpass_core::event::RunEvent;
    use cpass_core::{CourseRunExecutionState, CourseRunQueueEntryState};

    #[test]
    fn renders_course_run_lifecycle_from_events() {
        let mut state = RunTuiState::default();
        let events = [
            RunEvent::CourseRunPlanningStarted {
                course_id: Some(1001),
                course_index: None,
            },
            RunEvent::CourseRunPlanningFinished {
                course_id: 1001,
                chapters: 2,
                task_points: 3,
            },
            RunEvent::CourseRunExecutionStarted {
                course_id: 1001,
                total_entries: 3,
            },
            RunEvent::CourseRunQueueEntryStateChanged {
                course_id: 1001,
                queue_index: 0,
                state: CourseRunQueueEntryState::Running,
            },
            RunEvent::CourseRunDocumentProgressReported {
                course_id: 1001,
                queue_index: 0,
                success: true,
            },
            RunEvent::CourseRunChapterWorkSnapshotFetched {
                course_id: 1001,
                queue_index: 0,
                snapshot: cpass_core::ChapterWorkFormSnapshot {
                    title: "绪论测验".to_owned(),
                    work_answer_id: 99001,
                    total_question_num: 3,
                    work_relation_id: 88001,
                    full_score: "100".to_owned(),
                    enc_work: "enc-work-submit-001".to_owned(),
                    questions: vec![cpass_core::ChapterWorkQuestionSummary {
                        question_index: 0,
                        question_id: 700101,
                        question_type: 0,
                        question_type_label: "单选题".to_owned(),
                        question_kind: cpass_core::NormalizedQuestionKind::from_question_type(0)
                            .as_str()
                            .to_owned(),
                        prompt: "普通话以哪种方言为基础方言？".to_owned(),
                        options: vec![cpass_core::ExamQuestionOption {
                            key: "A".to_owned(),
                            value: "北方方言".to_owned(),
                            rich_content: None,
                        }],
                        blanks: Vec::new(),
                    }],
                },
            },
            RunEvent::CourseRunChapterWorkCandidateSelectionPrepared {
                course_id: 1001,
                queue_index: 0,
                work_answer_id: 99001,
                total_questions: 1,
                selected_questions: 1,
            },
            RunEvent::Warning {
                message: "queue entry 0 blocked".to_owned(),
            },
            RunEvent::CourseRunQueueEntryStateChanged {
                course_id: 1001,
                queue_index: 0,
                state: CourseRunQueueEntryState::Blocked,
            },
            RunEvent::CourseRunExecutionFinished {
                course_id: 1001,
                state: CourseRunExecutionState::Blocked,
                total_entries: 3,
                completed_entries: 0,
                blocked_entries: 1,
            },
        ];

        for event in events {
            state.apply_event(&event);
        }

        let snapshot = state.render_text_snapshot();
        assert!(
            snapshot.contains("Planning: finished for course 1001 (2 chapters, 3 task points)")
        );
        assert!(snapshot.contains("Execution: blocked for course 1001 (3 total entries)"));
        assert!(snapshot.contains("Queue: 0 completed, 1 blocked, 2 pending"));
        assert!(snapshot.contains("  [0] blocked"));
        assert!(snapshot.contains("  [1] pending"));
        assert!(snapshot.contains("Warnings:"));
        assert!(snapshot.contains("queue entry 0 blocked"));
        assert!(snapshot.contains("Recent events:"));
        assert!(snapshot.contains("document queue[0] reported success=true"));
        assert!(snapshot.contains("chapter_work queue[0] fetched snapshot 99001 (1 questions)"));
        assert!(snapshot.contains(
            "chapter_work queue[0] prepared candidate selections 1/1 for snapshot 99001"
        ));
        assert!(snapshot.contains("execution finished for course 1001"));
    }

    #[test]
    fn caps_recent_warning_and_event_history() {
        let mut state = RunTuiState::default();
        for index in 0..10 {
            state.apply_event(&RunEvent::Warning {
                message: format!("warn-{index}"),
            });
        }

        let snapshot = state.render_text_snapshot();
        assert!(!snapshot.contains("warn-0"));
        assert!(!snapshot.contains("warn-1"));
        assert!(snapshot.contains("warn-6"));
        assert!(snapshot.contains("warn-9"));
    }
}
