use serde::Serialize;

use crate::execution::{CourseRunQueueEntry, CourseRunTaskKind};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TaskExecutorKey {
    Video,
    Document,
    Live,
    ChapterWork,
}

impl TaskExecutorKey {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Document => "document",
            Self::Live => "live",
            Self::ChapterWork => "chapter_work",
        }
    }

    #[must_use]
    pub const fn module(self) -> &'static str {
        match self {
            Self::Video => "insertvideo",
            Self::Document => "insertdoc",
            Self::Live => "insertlive",
            Self::ChapterWork => "work",
        }
    }

    #[must_use]
    pub fn matches_task_kind(self, task_kind: &CourseRunTaskKind) -> bool {
        matches!(
            (self, task_kind),
            (Self::Video, CourseRunTaskKind::Video)
                | (Self::Document, CourseRunTaskKind::Document)
                | (Self::Live, CourseRunTaskKind::Live)
                | (Self::ChapterWork, CourseRunTaskKind::ChapterWork)
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct TaskExecutorRegistration {
    pub key: TaskExecutorKey,
    pub module: &'static str,
}

impl TaskExecutorRegistration {
    const fn new(key: TaskExecutorKey) -> Self {
        Self {
            key,
            module: key.module(),
        }
    }
}

const REGISTRATIONS: [TaskExecutorRegistration; 4] = [
    TaskExecutorRegistration::new(TaskExecutorKey::Video),
    TaskExecutorRegistration::new(TaskExecutorKey::Document),
    TaskExecutorRegistration::new(TaskExecutorKey::Live),
    TaskExecutorRegistration::new(TaskExecutorKey::ChapterWork),
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskExecutorResolution {
    Registered {
        registration: TaskExecutorRegistration,
    },
    UnsupportedModule {
        module: String,
        task_kind: CourseRunTaskKind,
    },
    InconsistentTaskKind {
        module: String,
        task_kind: CourseRunTaskKind,
        expected_key: TaskExecutorKey,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskExecutorQueueResolution<'a> {
    pub entry: &'a CourseRunQueueEntry,
    pub resolution: TaskExecutorResolution,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TaskExecutorRegistry;

impl TaskExecutorRegistry {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    #[must_use]
    pub fn registrations(&self) -> &'static [TaskExecutorRegistration] {
        &REGISTRATIONS
    }

    #[must_use]
    pub fn resolve(&self, entry: &CourseRunQueueEntry) -> TaskExecutorResolution {
        if let Some(registration) = self.registration_for_module(entry.module.as_str()) {
            if registration.key.matches_task_kind(&entry.task_kind) {
                return TaskExecutorResolution::Registered { registration };
            }

            return TaskExecutorResolution::InconsistentTaskKind {
                module: entry.module.clone(),
                task_kind: entry.task_kind.clone(),
                expected_key: registration.key,
            };
        }

        TaskExecutorResolution::UnsupportedModule {
            module: entry.module.clone(),
            task_kind: entry.task_kind.clone(),
        }
    }

    fn registration_for_module(&self, module: &str) -> Option<TaskExecutorRegistration> {
        self.registrations()
            .iter()
            .copied()
            .find(|registration| registration.module == module)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TaskExecutorKey, TaskExecutorRegistration, TaskExecutorRegistry, TaskExecutorResolution,
    };
    use crate::execution::{CourseRunQueueEntry, CourseRunTaskKind};

    fn sample_queue_entry(module: &str, task_kind: CourseRunTaskKind) -> CourseRunQueueEntry {
        CourseRunQueueEntry {
            queue_index: 0,
            chapter_id: 11,
            chapter_index: 1,
            chapter_name: "Warmup".to_owned(),
            chapter_label: "1".to_owned(),
            card_index: 0,
            point_index: 0,
            module: module.to_owned(),
            task_kind,
            title: "Sample task".to_owned(),
            resource_id: "resource-001".to_owned(),
            iframe_data: None,
            attachment_metadata: None,
            chapter_work_metadata: None,
        }
    }

    #[test]
    fn lists_registered_executor_modules_in_stable_order() {
        let registry = TaskExecutorRegistry::new();

        assert_eq!(
            registry.registrations(),
            &[
                TaskExecutorRegistration {
                    key: TaskExecutorKey::Video,
                    module: "insertvideo",
                },
                TaskExecutorRegistration {
                    key: TaskExecutorKey::Document,
                    module: "insertdoc",
                },
                TaskExecutorRegistration {
                    key: TaskExecutorKey::Live,
                    module: "insertlive",
                },
                TaskExecutorRegistration {
                    key: TaskExecutorKey::ChapterWork,
                    module: "work",
                },
            ]
        );
    }

    #[test]
    fn resolves_registered_queue_entries_by_module_and_task_kind() {
        let registry = TaskExecutorRegistry::new();

        assert_eq!(
            registry.resolve(&sample_queue_entry("insertvideo", CourseRunTaskKind::Video)),
            TaskExecutorResolution::Registered {
                registration: TaskExecutorRegistration {
                    key: TaskExecutorKey::Video,
                    module: "insertvideo",
                },
            }
        );
        assert_eq!(
            registry.resolve(&sample_queue_entry(
                "insertdoc",
                CourseRunTaskKind::Document
            )),
            TaskExecutorResolution::Registered {
                registration: TaskExecutorRegistration {
                    key: TaskExecutorKey::Document,
                    module: "insertdoc",
                },
            }
        );
        assert_eq!(
            registry.resolve(&sample_queue_entry("insertlive", CourseRunTaskKind::Live)),
            TaskExecutorResolution::Registered {
                registration: TaskExecutorRegistration {
                    key: TaskExecutorKey::Live,
                    module: "insertlive",
                },
            }
        );
        assert_eq!(
            registry.resolve(&sample_queue_entry("work", CourseRunTaskKind::ChapterWork)),
            TaskExecutorResolution::Registered {
                registration: TaskExecutorRegistration {
                    key: TaskExecutorKey::ChapterWork,
                    module: "work",
                },
            }
        );
    }

    #[test]
    fn fails_closed_for_unsupported_modules() {
        let registry = TaskExecutorRegistry::new();

        assert_eq!(
            registry.resolve(&sample_queue_entry(
                "insertaudio",
                CourseRunTaskKind::Unknown("insertaudio".to_owned()),
            )),
            TaskExecutorResolution::UnsupportedModule {
                module: "insertaudio".to_owned(),
                task_kind: CourseRunTaskKind::Unknown("insertaudio".to_owned()),
            }
        );
    }

    #[test]
    fn fails_closed_when_planned_task_kind_does_not_match_module() {
        let registry = TaskExecutorRegistry::new();

        assert_eq!(
            registry.resolve(&sample_queue_entry(
                "insertvideo",
                CourseRunTaskKind::Document
            )),
            TaskExecutorResolution::InconsistentTaskKind {
                module: "insertvideo".to_owned(),
                task_kind: CourseRunTaskKind::Document,
                expected_key: TaskExecutorKey::Video,
            }
        );
    }
}
