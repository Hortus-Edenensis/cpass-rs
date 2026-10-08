use serde::{Deserialize, Serialize};

use crate::models::ChapterWorkFormSnapshot;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CourseRunExecutionState {
    Pending,
    Running,
    Completed,
    Blocked,
}

impl CourseRunExecutionState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CourseRunQueueEntryState {
    Pending,
    Running,
    Completed,
    Blocked,
}

impl CourseRunQueueEntryState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RunEvent {
    DoctorStarted,
    DoctorFinished {
        checks: usize,
    },
    ConfigValidated {
        path: String,
        searchers: usize,
    },
    LoginStarted {
        phone: String,
    },
    LoginSucceeded {
        phone: String,
        puid: u64,
    },
    CoursesListed {
        count: usize,
    },
    TasksScanned {
        course_id: u64,
        chapters: usize,
    },
    CourseRunPlanningStarted {
        course_id: Option<u64>,
        course_index: Option<usize>,
    },
    CourseRunPlanningFinished {
        course_id: u64,
        chapters: usize,
        task_points: usize,
    },
    CourseRunExecutionStarted {
        course_id: u64,
        total_entries: usize,
    },
    CourseRunQueueEntryStateChanged {
        course_id: u64,
        queue_index: usize,
        state: CourseRunQueueEntryState,
    },
    CourseRunVideoProgressReported {
        course_id: u64,
        queue_index: usize,
        playing_time_secs: u64,
        duration_secs: u64,
        is_passed: bool,
    },
    CourseRunDocumentProgressReported {
        course_id: u64,
        queue_index: usize,
        success: bool,
    },
    CourseRunLiveProgressReported {
        course_id: u64,
        queue_index: usize,
        success: bool,
    },
    CourseRunChapterWorkSnapshotFetched {
        course_id: u64,
        queue_index: usize,
        snapshot: ChapterWorkFormSnapshot,
    },
    CourseRunChapterWorkCandidateSelectionPrepared {
        course_id: u64,
        queue_index: usize,
        work_answer_id: u64,
        total_questions: usize,
        selected_questions: usize,
    },
    CourseRunExecutionFinished {
        course_id: u64,
        state: CourseRunExecutionState,
        total_entries: usize,
        completed_entries: usize,
        blocked_entries: usize,
    },
    ExamRunPlanningStarted {
        course_id: Option<u64>,
        course_index: Option<usize>,
        exam_id: Option<u64>,
        exam_index: Option<usize>,
    },
    ExamRunPlanningFinished {
        course_id: u64,
        exam_id: u64,
        questions: usize,
    },
    ExamsExported {
        course_id: u64,
        exams: usize,
        output: String,
    },
    RetryScheduled {
        url: String,
        attempt: u32,
        reason: String,
    },
    Warning {
        message: String,
    },
    CommandNotImplemented {
        command: &'static str,
    },
}

pub trait RunEventSink: Send + Sync {
    fn emit(&self, event: RunEvent);
}

#[cfg(test)]
mod tests {
    use super::{CourseRunExecutionState, CourseRunQueueEntryState, RunEvent};
    use crate::models::{ChapterWorkFormSnapshot, ChapterWorkQuestionSummary, ExamQuestionOption};
    use crate::question_kind::NormalizedQuestionKind;
    use serde_json::json;

    #[test]
    fn serializes_course_run_runtime_events_with_snake_case_states() {
        let started = serde_json::to_value(RunEvent::CourseRunExecutionStarted {
            course_id: 1001,
            total_entries: 3,
        })
        .expect("serialize started event");
        let transitioned = serde_json::to_value(RunEvent::CourseRunQueueEntryStateChanged {
            course_id: 1001,
            queue_index: 1,
            state: CourseRunQueueEntryState::Running,
        })
        .expect("serialize queue transition event");
        let video_reported = serde_json::to_value(RunEvent::CourseRunVideoProgressReported {
            course_id: 1001,
            queue_index: 0,
            playing_time_secs: 602,
            duration_secs: 602,
            is_passed: true,
        })
        .expect("serialize video report event");
        let document_reported = serde_json::to_value(RunEvent::CourseRunDocumentProgressReported {
            course_id: 1001,
            queue_index: 2,
            success: true,
        })
        .expect("serialize document report event");
        let live_reported = serde_json::to_value(RunEvent::CourseRunLiveProgressReported {
            course_id: 1001,
            queue_index: 3,
            success: true,
        })
        .expect("serialize live report event");
        let chapter_work_snapshot =
            serde_json::to_value(RunEvent::CourseRunChapterWorkSnapshotFetched {
                course_id: 1001,
                queue_index: 1,
                snapshot: ChapterWorkFormSnapshot {
                    title: "绪论测验".to_owned(),
                    work_answer_id: 99001,
                    total_question_num: 3,
                    work_relation_id: 88001,
                    full_score: "100".to_owned(),
                    enc_work: "enc-work-submit-001".to_owned(),
                    questions: vec![ChapterWorkQuestionSummary {
                        question_index: 0,
                        question_id: 700101,
                        question_type: 0,
                        question_type_label: "单选题".to_owned(),
                        question_kind: NormalizedQuestionKind::from_question_type(0)
                            .as_str()
                            .to_owned(),
                        prompt: "普通话以哪种方言为基础方言？".to_owned(),
                        options: vec![
                            ExamQuestionOption {
                                key: "A".to_owned(),
                                value: "吴方言".to_owned(),
                                rich_content: None,
                            },
                            ExamQuestionOption {
                                key: "B".to_owned(),
                                value: "北方方言".to_owned(),
                                rich_content: None,
                            },
                        ],
                        blanks: Vec::new(),
                    }],
                },
            })
            .expect("serialize chapter work snapshot event");
        let chapter_work_candidate_selection =
            serde_json::to_value(RunEvent::CourseRunChapterWorkCandidateSelectionPrepared {
                course_id: 1001,
                queue_index: 1,
                work_answer_id: 99001,
                total_questions: 3,
                selected_questions: 2,
            })
            .expect("serialize chapter work candidate selection event");
        let finished = serde_json::to_value(RunEvent::CourseRunExecutionFinished {
            course_id: 1001,
            state: CourseRunExecutionState::Blocked,
            total_entries: 3,
            completed_entries: 1,
            blocked_entries: 1,
        })
        .expect("serialize finished event");

        assert_eq!(
            started,
            json!({
                "event": "course_run_execution_started",
                "course_id": 1001,
                "total_entries": 3
            })
        );
        assert_eq!(
            transitioned,
            json!({
                "event": "course_run_queue_entry_state_changed",
                "course_id": 1001,
                "queue_index": 1,
                "state": "running"
            })
        );
        assert_eq!(
            video_reported,
            json!({
                "event": "course_run_video_progress_reported",
                "course_id": 1001,
                "queue_index": 0,
                "playing_time_secs": 602,
                "duration_secs": 602,
                "is_passed": true
            })
        );
        assert_eq!(
            document_reported,
            json!({
                "event": "course_run_document_progress_reported",
                "course_id": 1001,
                "queue_index": 2,
                "success": true
            })
        );
        assert_eq!(
            live_reported,
            json!({
                "event": "course_run_live_progress_reported",
                "course_id": 1001,
                "queue_index": 3,
                "success": true
            })
        );
        assert_eq!(
            chapter_work_snapshot,
            json!({
                "event": "course_run_chapter_work_snapshot_fetched",
                "course_id": 1001,
                "queue_index": 1,
                "snapshot": {
                    "title": "绪论测验",
                    "work_answer_id": 99001,
                    "total_question_num": 3,
                    "work_relation_id": 88001,
                    "full_score": "100",
                    "enc_work": "enc-work-submit-001",
                    "questions": [
                        {
                            "question_index": 0,
                            "question_id": 700101,
                            "question_type": 0,
                            "question_type_label": "单选题",
                            "question_kind": "single_choice",
                            "prompt": "普通话以哪种方言为基础方言？",
                            "options": [
                                {
                                    "key": "A",
                                    "value": "吴方言"
                                },
                                {
                                    "key": "B",
                                    "value": "北方方言"
                                }
                            ]
                        }
                    ]
                }
            })
        );
        assert_eq!(
            chapter_work_candidate_selection,
            json!({
                "event": "course_run_chapter_work_candidate_selection_prepared",
                "course_id": 1001,
                "queue_index": 1,
                "work_answer_id": 99001,
                "total_questions": 3,
                "selected_questions": 2
            })
        );
        assert_eq!(
            finished,
            json!({
                "event": "course_run_execution_finished",
                "course_id": 1001,
                "state": "blocked",
                "total_entries": 3,
                "completed_entries": 1,
                "blocked_entries": 1
            })
        );
    }
}
