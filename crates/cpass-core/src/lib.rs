pub mod chaoxing;
pub mod config;
pub mod error;
pub mod event;
pub mod execution;
pub mod models;
pub mod question_kind;
pub mod searcher;
pub mod secret;
pub mod session;
pub mod task_executor;
pub mod transport;

pub use chaoxing::{
    ChaoxingClient, QrLoginBootstrap, QrLoginFlow, QrLoginPollOutcome, QrLoginStatus,
};
pub use config::{
    AppConfig, AppPaths, ConfigOutput, LoginConfig, NotificationConfig, SearcherConfig,
    TransportConfig,
};
pub use error::{CpassError, Result};
pub use event::{CourseRunExecutionState, CourseRunQueueEntryState, RunEvent, RunEventSink};
pub use execution::{
    ChapterWorkTaskExecutionContext, CourseRunChapterPlan, CourseRunExecutionPreflight,
    CourseRunExecutionPreflightEntry, CourseRunExecutionResult, CourseRunHeadlessDriver,
    CourseRunPlan, CourseRunQueueDriverOutcome, CourseRunQueueEntry, CourseRunQueueEntryExecutor,
    CourseRunQueueExecutionResult, CourseRunTarget, CourseRunTaskKind, CourseRunTaskPoint,
    CourseRunner, DocumentTaskExecutionContext, ExamRunPlan, ExamRunQuestionKind,
    ExamRunQuestionPlan, ExamRunTarget, ExamRunner, FailClosedCourseRunExecutor,
    HeadlessChapterWorkCourseRunExecutor, HeadlessDocumentCourseRunExecutor,
    HeadlessLiveCourseRunExecutor, HeadlessVideoCourseRunExecutor, VideoTaskExecutionContext,
};
pub use models::{
    AccountProfile, Chapter, ChapterWorkAttachmentMetadata, ChapterWorkFormSnapshot,
    ChapterWorkMetadata, ChapterWorkQuestionSummary, ChapterWorkRuntimeRequest, Course, CourseExam,
    DoctorCheck, DoctorReport, DocumentReadingReportAck, DocumentReadingReportRequest, ExamMeta,
    ExamPreviewExportManifest, ExamPreviewQuery, ExamQuestionOption, ExamQuestionSummary,
    ExportManifest, LiveProgressReportAck, LiveProgressReportRequest, QuestionRichContent,
    TaskAttachment, TaskAttachmentSnapshot, TaskPointAttachmentMetadata, TaskPointSummary,
    VideoAttachmentStatus, VideoPlayReportAck, VideoPlayReportRequest,
};
pub use question_kind::NormalizedQuestionKind;
pub use searcher::{
    AnswerCandidate, AnswerCandidateSelection, AnswerQuery, AnswerQuerySource, AnswerQuestionKind,
    ChapterWorkCandidateSelectionBatch, ChapterWorkQueryBatch, HttpSearcherMethod,
    HttpSearcherPayloadMode, HttpSearcherProvider, HttpSearcherRequest, HttpSearcherRequestPayload,
    HttpSearcherRequestTemplate, JsonSearcherProvider, OpenAiCompatibleMessage,
    OpenAiCompatibleMessageRole, OpenAiCompatibleRequest, OpenAiCompatibleRequestTemplate,
    OpenAiCompatibleResponse, OpenAiCompatibleResponseChoice, OpenAiCompatibleResponseMessage,
    OpenAiCompatibleSearcherProvider, SearcherPipeline, SearcherProvider, SqliteSearcherProvider,
    build_searcher_pipeline,
};
pub use secret::{EnvSecretSource, SecretSource};
pub use session::{FileSessionStore, SessionRecord, SessionStore};
pub use task_executor::{
    TaskExecutorKey, TaskExecutorQueueResolution, TaskExecutorRegistration, TaskExecutorRegistry,
    TaskExecutorResolution,
};
pub use transport::{
    ChaoxingTransport, FixtureChaoxingTransport, RequestBody, ReqwestChaoxingTransport,
    TransportRequest, TransportResponse,
};
