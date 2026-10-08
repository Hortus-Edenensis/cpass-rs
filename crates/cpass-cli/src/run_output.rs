use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use cpass_core::event::{RunEvent, RunEventSink};
use cpass_core::{
    CourseRunExecutionPreflight, CourseRunExecutionResult, CourseRunPlan, SessionRecord,
};
use serde::Serialize;

#[derive(Default, Clone)]
pub struct RunEventBuffer {
    events: Arc<Mutex<Vec<RunEvent>>>,
}

impl RunEventBuffer {
    #[must_use]
    pub fn snapshot(&self) -> Vec<RunEvent> {
        self.events.lock().expect("event buffer poisoned").clone()
    }

    #[must_use]
    pub fn adapt<T>(&self, payload: T) -> EventOutput<T> {
        EventOutput {
            payload,
            events: self.snapshot(),
        }
    }
}

impl RunEventSink for RunEventBuffer {
    fn emit(&self, event: RunEvent) {
        self.events
            .lock()
            .expect("event buffer poisoned")
            .push(event);
    }
}

#[derive(Debug, Serialize)]
pub struct EventOutput<T> {
    #[serde(flatten)]
    payload: T,
    events: Vec<RunEvent>,
}

#[derive(Debug, Serialize)]
pub struct LoginPayload {
    pub session_path: Option<PathBuf>,
    pub record: SessionRecord,
}

#[derive(Debug, Serialize)]
pub struct CourseRunPayload {
    #[serde(flatten)]
    pub plan: CourseRunPlan,
    pub execution_preflight: CourseRunExecutionPreflight,
    pub execution_result: CourseRunExecutionResult,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{CourseRunPayload, EventOutput, LoginPayload, RunEventBuffer};
    use cpass_core::event::{RunEvent, RunEventSink};
    use cpass_core::models::CookieSnapshot;
    use cpass_core::{
        CourseRunExecutionPreflight, CourseRunExecutionPreflightEntry, CourseRunExecutionResult,
        CourseRunPlan, CourseRunQueueEntry, CourseRunQueueExecutionResult, CourseRunTaskKind,
        SessionRecord, TaskExecutorKey, TaskExecutorRegistration, TaskExecutorResolution,
    };
    use cpass_core::{CourseRunExecutionState, CourseRunQueueEntryState};
    use serde_json::json;

    #[test]
    fn buffer_preserves_event_order() {
        let buffer = RunEventBuffer::default();
        buffer.emit(RunEvent::DoctorStarted);
        buffer.emit(RunEvent::DoctorFinished { checks: 2 });

        assert_eq!(
            buffer.snapshot(),
            vec![
                RunEvent::DoctorStarted,
                RunEvent::DoctorFinished { checks: 2 }
            ]
        );
    }

    #[test]
    fn adapter_flattens_payload_and_serializes_events() {
        let buffer = RunEventBuffer::default();
        buffer.emit(RunEvent::CourseRunPlanningStarted {
            course_id: Some(1001),
            course_index: None,
        });

        let output = buffer.adapt(CourseRunPayload {
            plan: CourseRunPlan {
                course: cpass_core::Course {
                    course_id: 1001,
                    class_id: 2001,
                    cpi: 3001,
                    key: 4001,
                    name: "Rust Automation".to_owned(),
                    teacher_name: "Tester".to_owned(),
                    state: "ongoing".to_owned(),
                },
                chapters: Vec::new(),
                execution_queue: vec![CourseRunQueueEntry {
                    queue_index: 0,
                    chapter_id: 11,
                    chapter_index: 1,
                    chapter_name: "Intro".to_owned(),
                    chapter_label: "1".to_owned(),
                    card_index: 0,
                    point_index: 0,
                    module: "insertvideo".to_owned(),
                    task_kind: CourseRunTaskKind::Video,
                    title: "Watch".to_owned(),
                    resource_id: "video-001".to_owned(),
                    iframe_data: None,
                    attachment_metadata: None,
                    chapter_work_metadata: None,
                }],
                total_task_points: 1,
            },
            execution_preflight: CourseRunExecutionPreflight {
                all_entries_registered: true,
                total_entries: 1,
                registered_entries: 1,
                blocked_entries: 0,
                queue: vec![CourseRunExecutionPreflightEntry {
                    queue_index: 0,
                    resolution: TaskExecutorResolution::Registered {
                        registration: TaskExecutorRegistration {
                            key: TaskExecutorKey::Video,
                            module: "insertvideo",
                        },
                    },
                }],
            },
            execution_result: CourseRunExecutionResult {
                course_id: 1001,
                state: CourseRunExecutionState::Blocked,
                total_entries: 1,
                completed_entries: 0,
                blocked_entries: 1,
                queue: vec![CourseRunQueueExecutionResult {
                    queue_index: 0,
                    state: CourseRunQueueEntryState::Blocked,
                    resolution: TaskExecutorResolution::Registered {
                        registration: TaskExecutorRegistration {
                            key: TaskExecutorKey::Video,
                            module: "insertvideo",
                        },
                    },
                }],
            },
        });

        let value = serde_json::to_value(output).expect("serialize output");
        assert_eq!(value["course"]["course_id"], 1001);
        assert_eq!(value["execution_queue"][0]["queue_index"], 0);
        assert_eq!(value["execution_preflight"]["all_entries_registered"], true);
        assert_eq!(value["execution_result"]["state"], "blocked");
        assert_eq!(value["execution_result"]["queue"][0]["state"], "blocked");
        assert_eq!(
            value["execution_preflight"]["queue"][0]["resolution"]["registration"]["key"],
            "video"
        );
        assert_eq!(value["events"][0]["event"], "course_run_planning_started");
    }

    #[test]
    fn adapter_reuses_login_payload_shape() {
        let output: EventOutput<LoginPayload> = RunEventBuffer::default().adapt(LoginPayload {
            session_path: Some(PathBuf::from("/tmp/session.json")),
            record: SessionRecord {
                schema: "cpass.session.v1".to_owned(),
                account: cpass_core::AccountProfile {
                    puid: 100,
                    name: "Tester".to_owned(),
                    phone: "13800138000".to_owned(),
                    school: "Rust University".to_owned(),
                    sex: Some("1".to_owned()),
                    student_id: Some("20260001".to_owned()),
                },
                cookies: CookieSnapshot::empty(),
                saved_at: chrono::Utc::now(),
            },
        });

        let value = serde_json::to_value(output).expect("serialize login output");
        assert_eq!(
            value,
            json!({
                "session_path": "/tmp/session.json",
                "record": {
                    "schema": "cpass.session.v1",
                    "account": {
                        "puid": 100,
                        "name": "Tester",
                        "phone": "13800138000",
                        "school": "Rust University",
                        "sex": "1",
                        "student_id": "20260001"
                    },
                    "cookies": {
                        "hosts": {}
                    },
                    "saved_at": value["record"]["saved_at"]
                },
                "events": []
            })
        );
    }
}
