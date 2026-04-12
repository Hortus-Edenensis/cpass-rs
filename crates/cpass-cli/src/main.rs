use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use cpass_core::config::ConfigOutput;
use cpass_core::event::{RunEvent, RunEventSink};
use cpass_core::models::{DoctorCheck, DoctorReport, ExamPreviewExportManifest, ExportManifest};
use cpass_core::{
    AppConfig, ChaoxingClient, CourseRunHeadlessDriver, CourseRunQueueDriverOutcome,
    CourseRunQueueEntry, CourseRunQueueEntryExecutor, CourseRunTarget, CourseRunner, CpassError,
    EnvSecretSource, ExamRunTarget, FileSessionStore, FixtureChaoxingTransport,
    HeadlessChapterWorkCourseRunExecutor, HeadlessDocumentCourseRunExecutor,
    HeadlessLiveCourseRunExecutor, HeadlessVideoCourseRunExecutor, QrLoginFlow, QrLoginPollOutcome,
    ReqwestChaoxingTransport, Result, SearcherPipeline, SessionRecord, SessionStore,
    TaskExecutorKey, TaskExecutorRegistration, TaskExecutorRegistry, build_searcher_pipeline,
};
use qrcode::QrCode;
use qrcode::types::Color;
use tokio::time::{Duration as TokioDuration, sleep};

mod notification;
mod run_output;
mod run_tui;

use notification::{NotificationCoordinator, dispatch_notification_plans};
use run_output::{CourseRunPayload, LoginPayload, RunEventBuffer};
use run_tui::RunTui;

struct CliCourseRunExecutor {
    video: HeadlessVideoCourseRunExecutor,
    document: HeadlessDocumentCourseRunExecutor,
    live: HeadlessLiveCourseRunExecutor,
    chapter_work: HeadlessChapterWorkCourseRunExecutor,
}

impl CliCourseRunExecutor {
    fn new(
        video: HeadlessVideoCourseRunExecutor,
        document: HeadlessDocumentCourseRunExecutor,
        live: HeadlessLiveCourseRunExecutor,
        chapter_work: HeadlessChapterWorkCourseRunExecutor,
    ) -> Self {
        Self {
            video,
            document,
            live,
            chapter_work,
        }
    }
}

#[async_trait::async_trait]
impl CourseRunQueueEntryExecutor for CliCourseRunExecutor {
    async fn execute_queue_entry(
        &self,
        entry: &CourseRunQueueEntry,
        registration: TaskExecutorRegistration,
    ) -> Result<CourseRunQueueDriverOutcome> {
        match registration.key {
            TaskExecutorKey::Video => self.video.execute_queue_entry(entry, registration).await,
            TaskExecutorKey::Document => {
                self.document.execute_queue_entry(entry, registration).await
            }
            TaskExecutorKey::Live => self.live.execute_queue_entry(entry, registration).await,
            TaskExecutorKey::ChapterWork => {
                self.chapter_work
                    .execute_queue_entry(entry, registration)
                    .await
            }
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "cpass", version, about = "Rust CLI for cpass-rs")]
struct Cli {
    #[arg(long, global = true, default_value = "config.yml")]
    config: PathBuf,
    #[arg(
        long,
        global = true,
        env = "CPASS_PROFILE",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    profile: Option<String>,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true, env = "CPASS_FIXTURE_DIR")]
    fixture_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Doctor,
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    Login(LoginArgs),
    Courses {
        #[command(subcommand)]
        command: CoursesCommands,
    },
    Tasks {
        #[command(subcommand)]
        command: TasksCommands,
    },
    Exam {
        #[command(subcommand)]
        command: ExamCommands,
    },
    Run(RunArgs),
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    Validate,
}

#[derive(Subcommand, Debug)]
enum CoursesCommands {
    List(SessionArgs),
    Show(CourseShowArgs),
}

#[derive(Subcommand, Debug)]
enum TasksCommands {
    Scan(TaskScanArgs),
}

#[derive(Subcommand, Debug)]
enum ExamCommands {
    Show(ExamShowArgs),
    Export(ExamExportArgs),
    Preview {
        #[command(subcommand)]
        command: ExamPreviewCommands,
    },
}

#[derive(Subcommand, Debug)]
enum ExamPreviewCommands {
    Export(ExamPreviewExportArgs),
}

#[derive(Args, Debug, Clone)]
struct SessionArgs {
    #[arg(long)]
    phone: Option<String>,
}

#[derive(Args, Debug)]
struct LoginArgs {
    #[arg(long)]
    phone: Option<String>,
    #[arg(long)]
    password: Option<String>,
    #[arg(long)]
    no_save: bool,
}

#[derive(Args, Debug)]
struct TaskScanArgs {
    #[command(flatten)]
    session: SessionArgs,
    #[arg(long, conflicts_with = "course_index")]
    course_id: Option<u64>,
    #[arg(long, conflicts_with = "course_id")]
    course_index: Option<usize>,
}

#[derive(Args, Debug)]
struct CourseShowArgs {
    #[command(flatten)]
    session: SessionArgs,
    #[arg(long, conflicts_with = "course_index")]
    course_id: Option<u64>,
    #[arg(long, conflicts_with = "course_id")]
    course_index: Option<usize>,
}

#[derive(Args, Debug)]
struct ExamExportArgs {
    #[command(flatten)]
    session: SessionArgs,
    #[arg(long, conflicts_with = "course_index")]
    course_id: Option<u64>,
    #[arg(long, conflicts_with = "course_id")]
    course_index: Option<usize>,
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct ExamShowArgs {
    #[command(flatten)]
    session: SessionArgs,
    #[arg(long, conflicts_with = "course_index")]
    course_id: Option<u64>,
    #[arg(long, conflicts_with = "course_id")]
    course_index: Option<usize>,
    #[arg(long, conflicts_with = "exam_index")]
    exam_id: Option<u64>,
    #[arg(long, conflicts_with = "exam_id")]
    exam_index: Option<usize>,
}

#[derive(Args, Debug)]
struct ExamPreviewExportArgs {
    #[command(flatten)]
    session: SessionArgs,
    #[arg(long, conflicts_with = "course_index")]
    course_id: Option<u64>,
    #[arg(long, conflicts_with = "course_id")]
    course_index: Option<usize>,
    #[arg(long, conflicts_with = "exam_index")]
    exam_id: Option<u64>,
    #[arg(long, conflicts_with = "exam_id")]
    exam_index: Option<usize>,
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct RunArgs {
    #[command(flatten)]
    session: SessionArgs,
    #[arg(long, conflicts_with = "course_index")]
    course_id: Option<u64>,
    #[arg(long, conflicts_with = "course_id")]
    course_index: Option<usize>,
    #[arg(long)]
    tui: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(error) = run(cli).await {
        eprintln!("error: {error}");
        std::process::exit(match error {
            CpassError::UnsupportedCommand(_) => 2,
            _ => 1,
        });
    }
}

async fn run(cli: Cli) -> Result<()> {
    let secrets = EnvSecretSource;
    let config_path = cli.config.clone();
    let selected_profile = cli.profile.clone();
    let json_output = cli.json;
    let fixture_dir = cli.fixture_dir.clone();
    match cli.command {
        None => {
            if json_output {
                return Err(CpassError::Validation(
                    "--json cannot be used when launching the default TUI".to_owned(),
                ));
            }
            launch_default_interactive(
                &config_path,
                selected_profile.as_deref(),
                fixture_dir.as_deref(),
                &secrets,
            )
            .await
        }
        Some(Commands::Doctor) => {
            let config = load_config(&config_path, selected_profile.as_deref(), &secrets)?;
            let output = doctor(
                &config_path,
                selected_profile.as_deref(),
                &config,
                fixture_dir.as_deref(),
            )?;
            let notifications = NotificationCoordinator::new(&config.notifications);
            let sink = notifications.wrap_sink(Arc::new(RunEventBuffer::default()));
            sink.emit(RunEvent::DoctorFinished {
                checks: output.checks.len(),
            });
            dispatch_notification_plans(
                &notifications.plans(),
                config_path.parent().unwrap_or(std::path::Path::new(".")),
                Duration::from_secs(config.transport.timeout_secs),
            )
            .await;
            print_output(json_output, &output)
        }
        Some(Commands::Config {
            command: ConfigCommands::Validate,
        }) => {
            let config = load_config(&config_path, selected_profile.as_deref(), &secrets)?;
            let notifications = NotificationCoordinator::new(&config.notifications);
            let sink = notifications.wrap_sink(Arc::new(RunEventBuffer::default()));
            sink.emit(RunEvent::ConfigValidated {
                path: config_path.display().to_string(),
                searchers: config.searchers.len(),
            });
            dispatch_notification_plans(
                &notifications.plans(),
                config_path.parent().unwrap_or(std::path::Path::new(".")),
                Duration::from_secs(config.transport.timeout_secs),
            )
            .await;
            let output = ConfigOutput {
                config_path: config_path.clone(),
                selected_profile,
                automation_paths: normalized_automation_paths(&config_path, &config.paths)?,
                config,
            };
            print_output(json_output, &output)
        }
        Some(Commands::Login(args)) => {
            let config = load_config(&config_path, selected_profile.as_deref(), &secrets)?;
            let event_buffer = Arc::new(RunEventBuffer::default());
            let notifications = NotificationCoordinator::new(&config.notifications);
            let sink = notifications.wrap_sink(event_buffer.clone());
            let phone = args
                .phone
                .or_else(|| config.login.phone.clone())
                .ok_or(CpassError::MissingCredentials)?;
            let password = args
                .password
                .or_else(|| config.login.password.clone())
                .ok_or(CpassError::MissingCredentials)?;

            sink.emit(RunEvent::LoginStarted {
                phone: phone.clone(),
            });
            let (client, account, snapshot) =
                ChaoxingClient::login_password(&phone, &password, config.transport.clone()).await?;
            drop(client);
            sink.emit(RunEvent::LoginSucceeded {
                phone: account.phone.clone(),
                puid: account.puid,
            });

            let record = SessionRecord {
                schema: "cpass.session.v1".to_owned(),
                account,
                cookies: snapshot,
                saved_at: chrono::Utc::now(),
            };

            let session_path = if args.no_save {
                None
            } else {
                let store = FileSessionStore::new(config.paths.session_dir.clone());
                Some(store.save(&record)?)
            };

            dispatch_notification_plans(
                &notifications.plans(),
                config_path.parent().unwrap_or(std::path::Path::new(".")),
                Duration::from_secs(config.transport.timeout_secs),
            )
            .await;
            let output = event_buffer.adapt(LoginPayload {
                session_path,
                record,
            });
            print_output(json_output, &output)
        }
        Some(Commands::Courses {
            command: CoursesCommands::List(args),
        }) => {
            let config = load_config(&config_path, selected_profile.as_deref(), &secrets)?;
            let (client, _account) =
                client_from_session(&config, args.phone.as_deref(), fixture_dir.as_deref()).await?;
            let courses = client.fetch_courses().await?;
            print_output(json_output, &courses)
        }
        Some(Commands::Courses {
            command: CoursesCommands::Show(args),
        }) => {
            let config = load_config(&config_path, selected_profile.as_deref(), &secrets)?;
            let (client, _account) = client_from_session(
                &config,
                args.session.phone.as_deref(),
                fixture_dir.as_deref(),
            )
            .await?;
            let target = CourseRunTarget::new(args.course_id, args.course_index)?;
            let course = target.resolve_course(client.fetch_courses().await?)?;
            print_output(json_output, &course)
        }
        Some(Commands::Tasks {
            command: TasksCommands::Scan(args),
        }) => {
            let config = load_config(&config_path, selected_profile.as_deref(), &secrets)?;
            let (client, _account) = client_from_session(
                &config,
                args.session.phone.as_deref(),
                fixture_dir.as_deref(),
            )
            .await?;
            let target = CourseRunTarget::new(args.course_id, args.course_index)?;
            let course = target.resolve_course(client.fetch_courses().await?)?;
            let chapters = client
                .fetch_task_scan_with_attachment_metadata(&course)
                .await?;
            #[derive(serde::Serialize)]
            struct ScanOutput {
                course: cpass_core::Course,
                chapters: Vec<cpass_core::Chapter>,
            }
            print_output(json_output, &ScanOutput { course, chapters })
        }
        Some(Commands::Exam {
            command: ExamCommands::Show(args),
        }) => {
            let config = load_config(&config_path, selected_profile.as_deref(), &secrets)?;
            let (client, account) = client_from_session(
                &config,
                args.session.phone.as_deref(),
                fixture_dir.as_deref(),
            )
            .await?;
            let target = ExamRunTarget::new(
                args.course_id,
                args.course_index,
                args.exam_id,
                args.exam_index,
            )?;
            let course = target.resolve_course(client.fetch_courses().await?)?;
            let exams = client.fetch_exams(&course).await?;
            let mut exam = target.resolve_exam(exams)?;
            let meta = client.fetch_exam_meta(&course, &account, &exam).await?;
            exam.meta = Some(meta);
            #[derive(serde::Serialize)]
            struct ExamShowOutput {
                course: cpass_core::Course,
                exam: cpass_core::CourseExam,
            }
            print_output(json_output, &ExamShowOutput { course, exam })
        }
        Some(Commands::Exam {
            command: ExamCommands::Export(args),
        }) => {
            let config = load_config(&config_path, selected_profile.as_deref(), &secrets)?;
            let (client, account) = client_from_session(
                &config,
                args.session.phone.as_deref(),
                fixture_dir.as_deref(),
            )
            .await?;
            let target = CourseRunTarget::new(args.course_id, args.course_index)?;
            let course = target.resolve_course(client.fetch_courses().await?)?;
            let mut exams = client.fetch_exams(&course).await?;
            for exam in &mut exams {
                if let Ok(meta) = client.fetch_exam_meta(&course, &account, exam).await {
                    exam.meta = Some(meta);
                }
            }
            let output_path = args.output.unwrap_or_else(|| {
                config
                    .paths
                    .export_dir
                    .join(format!("exam_catalog_{}.json", course.course_id))
            });
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let manifest = ExportManifest {
                generated_at: chrono::Utc::now(),
                account,
                course: course.clone(),
                exams,
                output_path: output_path.clone(),
            };
            std::fs::write(&output_path, serde_json::to_string_pretty(&manifest)?)?;
            print_output(json_output, &manifest)
        }
        Some(Commands::Exam {
            command:
                ExamCommands::Preview {
                    command: ExamPreviewCommands::Export(args),
                },
        }) => {
            let config = load_config(&config_path, selected_profile.as_deref(), &secrets)?;
            let (client, account) = client_from_session(
                &config,
                args.session.phone.as_deref(),
                fixture_dir.as_deref(),
            )
            .await?;
            let target = ExamRunTarget::new(
                args.course_id,
                args.course_index,
                args.exam_id,
                args.exam_index,
            )?;
            let course = target.resolve_course(client.fetch_courses().await?)?;
            let exams = client.fetch_exams(&course).await?;
            let mut exam = target.resolve_exam(exams)?;
            let meta = client.fetch_exam_meta(&course, &account, &exam).await?;
            let preview = cpass_core::ExamPreviewQuery::from_exam_meta(&meta).ok_or_else(|| {
                CpassError::Validation(
                    "exam preview export requires exam_answer_id from read-only exam cover metadata"
                        .to_owned(),
                )
            })?;
            exam.meta = Some(meta);
            let questions = client
                .fetch_exam_preview_questions(&course, &exam, &preview)
                .await?;
            let output_path = args.output.unwrap_or_else(|| {
                config.paths.export_dir.join(format!(
                    "exam_preview_{}_{}.json",
                    course.course_id, exam.exam_id
                ))
            });
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let manifest = ExamPreviewExportManifest {
                generated_at: chrono::Utc::now(),
                account,
                course,
                exam,
                preview,
                questions,
                output_path: output_path.clone(),
            };
            std::fs::write(&output_path, serde_json::to_string_pretty(&manifest)?)?;
            print_output(json_output, &manifest)
        }
        Some(Commands::Run(args)) => {
            let config = load_config(&config_path, selected_profile.as_deref(), &secrets)?;
            let searcher_pipeline = build_searcher_pipeline(
                &config.searchers,
                config_path.parent().unwrap_or(std::path::Path::new(".")),
            )?
            .map(Arc::new);
            let target = CourseRunTarget::new(args.course_id, args.course_index)?;
            if args.tui {
                if json_output {
                    return Err(CpassError::Validation(
                        "--tui cannot be combined with --json".to_owned(),
                    ));
                }
                let tui = Arc::new(RunTui::new()?);
                let notifications = NotificationCoordinator::new(&config.notifications);
                let sink = notifications.wrap_sink(tui.clone());
                let result = execute_course_run(
                    &config,
                    args.session.phone.as_deref(),
                    fixture_dir.as_deref(),
                    searcher_pipeline,
                    sink.clone(),
                    target,
                )
                .await;
                let notification_plans = notifications.plans();
                tui.finish()?;
                dispatch_notification_plans(
                    &notification_plans,
                    config_path.parent().unwrap_or(std::path::Path::new(".")),
                    Duration::from_secs(config.transport.timeout_secs),
                )
                .await;
                result.map(|_| ())
            } else {
                let event_buffer = Arc::new(RunEventBuffer::default());
                let notifications = NotificationCoordinator::new(&config.notifications);
                let sink = notifications.wrap_sink(event_buffer.clone());
                let output = execute_course_run(
                    &config,
                    args.session.phone.as_deref(),
                    fixture_dir.as_deref(),
                    searcher_pipeline,
                    sink.clone(),
                    target,
                )
                .await?;
                dispatch_notification_plans(
                    &notifications.plans(),
                    config_path.parent().unwrap_or(std::path::Path::new(".")),
                    Duration::from_secs(config.transport.timeout_secs),
                )
                .await;
                let output = event_buffer.adapt(output);
                print_output(json_output, &output)
            }
        }
    }
}

struct InteractiveAuthResult {
    selected_phone: String,
    client: ChaoxingClient,
}

enum InteractiveNextStep {
    Quit,
    RunCourse(CourseRunTarget),
}

async fn launch_default_interactive(
    config_path: &Path,
    selected_profile: Option<&str>,
    fixture_dir: Option<&Path>,
    secrets: &dyn cpass_core::SecretSource,
) -> Result<()> {
    let config = load_config(config_path, selected_profile, secrets)?;
    print_interactive_banner()?;
    let Some(auth) = resolve_interactive_auth(&config, fixture_dir).await? else {
        return Ok(());
    };
    let searcher_pipeline = build_searcher_pipeline(
        &config.searchers,
        config_path.parent().unwrap_or(Path::new(".")),
    )?
    .map(Arc::new);
    let next_step = prompt_interactive_next_step(&auth.client).await?;
    let target = match next_step {
        InteractiveNextStep::Quit => return Ok(()),
        InteractiveNextStep::RunCourse(target) => target,
    };
    let tui = Arc::new(RunTui::new()?);
    let notifications = NotificationCoordinator::new(&config.notifications);
    let sink = notifications.wrap_sink(tui.clone());
    let result = execute_course_run(
        &config,
        Some(auth.selected_phone.as_str()),
        fixture_dir,
        searcher_pipeline,
        sink.clone(),
        target,
    )
    .await;
    let notification_plans = notifications.plans();
    tui.finish()?;
    dispatch_notification_plans(
        &notification_plans,
        config_path.parent().unwrap_or(Path::new(".")),
        Duration::from_secs(config.transport.timeout_secs),
    )
    .await;
    result.map(|_| ())
}

fn print_interactive_banner() -> Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "cpass interactive launcher")?;
    writeln!(stdout, "=========================")?;
    writeln!(stdout, "Version: {}", env!("CARGO_PKG_VERSION"))?;
    writeln!(stdout)?;
    stdout.flush()?;
    Ok(())
}

async fn resolve_interactive_auth(
    config: &AppConfig,
    fixture_dir: Option<&Path>,
) -> Result<Option<InteractiveAuthResult>> {
    let store = FileSessionStore::new(config.paths.session_dir.clone());
    let mut sessions = store.list()?;
    sessions.sort_by(|left, right| right.saved_at.cmp(&left.saved_at));

    if sessions.is_empty() {
        return prompt_interactive_login(config, fixture_dir).await;
    }

    if sessions.len() == 1 {
        let selected_phone = sessions[0].account.phone.clone();
        return interactive_auth_from_selected_session(
            config,
            fixture_dir,
            selected_phone.as_str(),
        )
        .await;
    }

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "可用会话:")?;
    for (index, session) in sessions.iter().enumerate() {
        writeln!(
            stdout,
            "  [{index}] {} {} [puid={}]",
            session.account.phone, session.account.name, session.account.puid
        )?;
    }
    write!(stdout, "\n选择会话序号，留空登录新账号，输入 q 退出: ")?;
    stdout.flush()?;
    drop(stdout);

    let selection = read_line_trimmed()?;
    if selection.eq_ignore_ascii_case("q") {
        return Ok(None);
    }
    if selection.is_empty() {
        return prompt_interactive_login(config, fixture_dir).await;
    }
    let index = selection.parse::<usize>().map_err(|_| {
        CpassError::Validation(format!(
            "invalid session selection `{selection}`; enter one of the listed session indexes"
        ))
    })?;
    let session = sessions
        .get(index)
        .ok_or_else(|| CpassError::Validation(format!("session index {index} not found")))?;
    let selected_phone = session.account.phone.clone();
    interactive_auth_from_selected_session(config, fixture_dir, selected_phone.as_str()).await
}

async fn interactive_auth_from_selected_session(
    config: &AppConfig,
    fixture_dir: Option<&Path>,
    selected_phone: &str,
) -> Result<Option<InteractiveAuthResult>> {
    let (client, _account) = client_from_session(config, Some(selected_phone), fixture_dir).await?;

    if client.fetch_account_info().await.is_err() {
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "会话已失效，请重新登录。")?;
        stdout.flush()?;
        drop(stdout);
        return prompt_interactive_login(config, fixture_dir).await;
    }

    Ok(Some(InteractiveAuthResult {
        selected_phone: selected_phone.to_owned(),
        client,
    }))
}

async fn prompt_interactive_login(
    config: &AppConfig,
    fixture_dir: Option<&Path>,
) -> Result<Option<InteractiveAuthResult>> {
    loop {
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "会话存档为空，请登录账号。")?;
        writeln!(stdout, "请输入手机号，留空为二维码登录；输入 q 退出。")?;
        write!(stdout, "手机号: ")?;
        stdout.flush()?;
        drop(stdout);

        let phone = read_line_trimmed()?;
        if phone.eq_ignore_ascii_case("q") {
            return Ok(None);
        }

        if phone.is_empty() {
            match prompt_interactive_qr_login(config, fixture_dir).await {
                Ok(auth) => return Ok(Some(auth)),
                Err(CpassError::LoginFailed(message)) => {
                    let mut stdout = io::stdout().lock();
                    writeln!(stdout, "二维码登录失败：{message}")?;
                    stdout.flush()?;
                    continue;
                }
                Err(other) => return Err(other),
            }
        }

        let password = prompt_password("密码: ")?;
        match ChaoxingClient::login_password(&phone, &password, config.transport.clone()).await {
            Ok((client, account, snapshot)) => {
                let record = SessionRecord {
                    schema: "cpass.session.v1".to_owned(),
                    account: account.clone(),
                    cookies: snapshot,
                    saved_at: chrono::Utc::now(),
                };
                let store = FileSessionStore::new(config.paths.session_dir.clone());
                store.save(&record)?;
                return Ok(Some(InteractiveAuthResult {
                    selected_phone: account.phone.clone(),
                    client,
                }));
            }
            Err(CpassError::LoginFailed(message)) => {
                let mut stdout = io::stdout().lock();
                writeln!(stdout, "密码登录失败：{message}")?;
                stdout.flush()?;
            }
            Err(other) => return Err(other),
        }
    }
}

async fn prompt_interactive_qr_login(
    config: &AppConfig,
    fixture_dir: Option<&Path>,
) -> Result<InteractiveAuthResult> {
    let flow = if let Some(fixture_dir) = fixture_dir {
        QrLoginFlow::begin_with_transport(
            Arc::new(FixtureChaoxingTransport::new(fixture_dir.to_path_buf())),
            None,
        )
        .await?
    } else {
        ChaoxingClient::begin_qr_login(config.transport.clone()).await?
    };

    let qr_url = flow.qr_url();
    let qr_render = render_qr_text(&qr_url);
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "请使用学习通扫码登录：")?;
    writeln!(stdout, "{qr_render}")?;
    writeln!(stdout, "二维码内容 URL: {qr_url}")?;
    writeln!(stdout, "等待扫码确认...")?;
    stdout.flush()?;
    drop(stdout);

    let mut announced_scan = false;
    loop {
        match flow.poll().await? {
            QrLoginPollOutcome::Pending => sleep(TokioDuration::from_secs(1)).await,
            QrLoginPollOutcome::Scanned { nickname, uid } => {
                if !announced_scan {
                    let mut stdout = io::stdout().lock();
                    writeln!(stdout, "二维码已扫描：nickname={nickname} uid={uid}")?;
                    stdout.flush()?;
                    announced_scan = true;
                }
                sleep(TokioDuration::from_secs(1)).await;
            }
            QrLoginPollOutcome::Success {
                client,
                account,
                snapshot,
            } => {
                let record = SessionRecord {
                    schema: "cpass.session.v1".to_owned(),
                    account: account.clone(),
                    cookies: snapshot,
                    saved_at: chrono::Utc::now(),
                };
                let store = FileSessionStore::new(config.paths.session_dir.clone());
                store.save(&record)?;
                return Ok(InteractiveAuthResult {
                    selected_phone: account.phone.clone(),
                    client,
                });
            }
            QrLoginPollOutcome::Expired => {
                return Err(CpassError::LoginFailed("二维码已失效，请重试".to_owned()));
            }
            QrLoginPollOutcome::Failed { message } => {
                return Err(CpassError::LoginFailed(message));
            }
        }
    }
}

fn render_qr_text(content: &str) -> String {
    match QrCode::new(content.as_bytes()) {
        Ok(code) => {
            let quiet_zone = 2_usize;
            let width = code.width();
            let colors = code.to_colors();
            let light_bg = "\x1b[107m";
            let dark_bg = "\x1b[40m";
            let reset = "\x1b[0m";
            let module = "  ";
            let full_width = width + quiet_zone * 2;
            let blank_line = format!("{light_bg}{}{reset}", module.repeat(full_width));
            let mut lines = Vec::with_capacity(width + quiet_zone * 2);

            for _ in 0..quiet_zone {
                lines.push(blank_line.clone());
            }

            for y in 0..width {
                let mut line = String::new();
                let mut current_dark: Option<bool> = None;

                let push_module =
                    |is_dark: bool, line: &mut String, current_dark: &mut Option<bool>| {
                        if current_dark.as_ref() != Some(&is_dark) {
                            line.push_str(if is_dark { dark_bg } else { light_bg });
                            *current_dark = Some(is_dark);
                        }
                        line.push_str(module);
                    };

                for _ in 0..quiet_zone {
                    push_module(false, &mut line, &mut current_dark);
                }
                for x in 0..width {
                    match colors[y * width + x] {
                        Color::Dark => push_module(true, &mut line, &mut current_dark),
                        Color::Light => push_module(false, &mut line, &mut current_dark),
                    }
                }
                for _ in 0..quiet_zone {
                    push_module(false, &mut line, &mut current_dark);
                }
                line.push_str(reset);
                lines.push(line);
            }

            for _ in 0..quiet_zone {
                lines.push(blank_line.clone());
            }

            lines.join("\n")
        }
        Err(_) => format!("(二维码渲染失败，请复制此 URL 自行生成二维码)\n{content}"),
    }
}

async fn prompt_interactive_next_step(client: &ChaoxingClient) -> Result<InteractiveNextStep> {
    let courses = client.fetch_courses().await?;
    if courses.is_empty() {
        return Err(CpassError::Validation(
            "no courses available for the interactive launcher".to_owned(),
        ));
    }

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "课程列表:")?;
    for (index, course) in courses.iter().enumerate() {
        writeln!(
            stdout,
            "  [{index}] {} ({}) [course_id={}] state={}",
            course.name, course.teacher_name, course.course_id, course.state
        )?;
    }
    writeln!(
        stdout,
        "\n输入课程序号 / 课程名 / course_id，前缀 EXAM| 进入考试路由，输入 q 退出。"
    )?;
    write!(stdout, "选择课程: ")?;
    stdout.flush()?;
    drop(stdout);

    let mut selection = String::new();
    io::stdin().read_line(&mut selection)?;
    let trimmed = selection.trim();
    if trimmed.eq_ignore_ascii_case("q") {
        return Ok(InteractiveNextStep::Quit);
    }
    if trimmed.is_empty() {
        return Err(CpassError::Validation(
            "course selection is required when launching the interactive app".to_owned(),
        ));
    }

    if let Some(selector) = trimmed
        .strip_prefix("EXAM|")
        .or_else(|| trimmed.strip_prefix("exam|"))
    {
        let _ = resolve_interactive_course(selector.trim(), &courses)?;
        return Err(CpassError::UnsupportedCommand(
            "interactive EXAM| routing is not implemented yet; use `cpass exam ...` subcommands for now",
        ));
    }

    let course = resolve_interactive_course(trimmed, &courses)?;
    Ok(InteractiveNextStep::RunCourse(CourseRunTarget::new(
        Some(course.course_id),
        None,
    )?))
}

fn resolve_interactive_course<'a>(
    selector: &str,
    courses: &'a [cpass_core::Course],
) -> Result<&'a cpass_core::Course> {
    if let Ok(index) = selector.parse::<usize>()
        && let Some(course) = courses.get(index)
    {
        return Ok(course);
    }

    if let Ok(course_id) = selector.parse::<u64>()
        && let Some(course) = courses.iter().find(|course| course.course_id == course_id)
    {
        return Ok(course);
    }

    if let Some(course) = courses
        .iter()
        .find(|course| course.name.eq_ignore_ascii_case(selector))
    {
        return Ok(course);
    }

    let lowered = selector.to_lowercase();
    let mut matches = courses
        .iter()
        .filter(|course| course.name.to_lowercase().contains(&lowered));
    match (matches.next(), matches.next()) {
        (Some(course), None) => Ok(course),
        (Some(_), Some(_)) => Err(CpassError::Validation(format!(
            "course selection `{selector}` is ambiguous; use a course index or exact course_id"
        ))),
        _ => Err(CpassError::Validation(format!(
            "course selection `{selector}` did not match any course"
        ))),
    }
}

fn read_line_trimmed() -> Result<String> {
    let mut buffer = String::new();
    io::stdin().read_line(&mut buffer)?;
    Ok(buffer.trim().to_owned())
}

fn prompt_password(prompt: &str) -> Result<String> {
    let mut stdout = io::stdout().lock();
    write!(stdout, "{prompt}")?;
    stdout.flush()?;
    drop(stdout);

    #[cfg(unix)]
    let _ = std::process::Command::new("stty").arg("-echo").status();

    let mut password = String::new();
    io::stdin().read_line(&mut password)?;

    #[cfg(unix)]
    let _ = std::process::Command::new("stty").arg("echo").status();

    let mut stdout = io::stdout().lock();
    writeln!(stdout)?;
    stdout.flush()?;
    Ok(password.trim_end_matches(['\r', '\n']).to_owned())
}

fn load_config(
    config_path: &Path,
    selected_profile: Option<&str>,
    secrets: &dyn cpass_core::SecretSource,
) -> Result<AppConfig> {
    AppConfig::load_from_path_with_profile(config_path, secrets, selected_profile)
}

async fn execute_course_run(
    config: &AppConfig,
    phone: Option<&str>,
    fixture_dir: Option<&std::path::Path>,
    searcher_pipeline: Option<Arc<SearcherPipeline>>,
    sink: Arc<dyn RunEventSink>,
    target: CourseRunTarget,
) -> Result<CourseRunPayload> {
    let registry = TaskExecutorRegistry::new();
    let (client, account) = client_from_session(config, phone, fixture_dir).await?;
    let runner = CourseRunner::new(&client).with_sink(sink.clone());
    let plan = runner.build_plan(target).await?;
    let execution_preflight = plan.build_execution_preflight(&registry);
    let driver = CourseRunHeadlessDriver::new(&registry).with_sink(sink.clone());
    let executor = CliCourseRunExecutor::new(
        HeadlessVideoCourseRunExecutor::new(client.clone(), plan.course.clone(), account.clone())
            .with_sink(sink.clone()),
        HeadlessDocumentCourseRunExecutor::new(client.clone(), plan.course.clone())
            .with_sink(sink.clone()),
        HeadlessLiveCourseRunExecutor::new(client.clone(), plan.course.clone(), account.clone())
            .with_sink(sink.clone()),
        match searcher_pipeline {
            Some(searcher_pipeline) => HeadlessChapterWorkCourseRunExecutor::new(
                client.clone(),
                plan.course.clone(),
                account,
            )
            .with_searcher_pipeline(searcher_pipeline)
            .with_sink(sink),
            None => HeadlessChapterWorkCourseRunExecutor::new(
                client.clone(),
                plan.course.clone(),
                account,
            )
            .with_sink(sink),
        },
    );
    let execution_result = driver.drive(&plan, &executor).await?;

    Ok(CourseRunPayload {
        plan,
        execution_preflight,
        execution_result,
    })
}

fn doctor(
    config_path: &Path,
    selected_profile: Option<&str>,
    config: &AppConfig,
    fixture_dir: Option<&std::path::Path>,
) -> Result<DoctorReport> {
    let store = FileSessionStore::new(config.paths.session_dir.clone());
    let session_count = store.list()?.len();

    let mut checks = vec![
        DoctorCheck {
            name: "config".to_owned(),
            status: "ok".to_owned(),
            detail: format!("loaded {}", config_path.display()),
        },
        DoctorCheck {
            name: "transport".to_owned(),
            status: "ok".to_owned(),
            detail: format!(
                "timeout={}s retries={}",
                config.transport.timeout_secs, config.transport.retries
            ),
        },
        DoctorCheck {
            name: "sessions".to_owned(),
            status: if session_count > 0 { "ok" } else { "warn" }.to_owned(),
            detail: format!("{session_count} session file(s) detected"),
        },
        DoctorCheck {
            name: "env".to_owned(),
            status: "ok".to_owned(),
            detail: format!(
                "phone={} password={}",
                std::env::var("CPASS_PHONE").is_ok(),
                std::env::var("CPASS_PASSWORD").is_ok()
            ),
        },
    ];
    if let Some(fixture_dir) = fixture_dir {
        checks.push(DoctorCheck {
            name: "fixtures".to_owned(),
            status: "ok".to_owned(),
            detail: format!("offline fixture mode: {}", fixture_dir.display()),
        });
    }

    Ok(DoctorReport {
        config_path: config_path.to_path_buf(),
        selected_profile: selected_profile.map(str::to_owned),
        automation_paths: normalized_automation_paths(config_path, &config.paths)?,
        session_dir: config.paths.session_dir.clone(),
        log_dir: config.paths.log_dir.clone(),
        export_dir: config.paths.export_dir.clone(),
        face_dir: config.paths.face_dir.clone(),
        searcher_count: config.searchers.len(),
        checks,
    })
}

fn normalized_automation_paths(
    config_path: &Path,
    paths: &cpass_core::AppPaths,
) -> Result<cpass_core::AppPaths> {
    let base_dir = normalized_config_base_dir(config_path)?;
    Ok(cpass_core::AppPaths {
        session_dir: normalize_output_path(&base_dir, &paths.session_dir),
        log_dir: normalize_output_path(&base_dir, &paths.log_dir),
        export_dir: normalize_output_path(&base_dir, &paths.export_dir),
        face_dir: normalize_output_path(&base_dir, &paths.face_dir),
    })
}

fn normalized_config_base_dir(config_path: &Path) -> Result<PathBuf> {
    let base_dir = config_path.parent().unwrap_or(std::path::Path::new("."));
    let joined = if base_dir.is_absolute() {
        base_dir.to_path_buf()
    } else {
        std::env::current_dir()?.join(base_dir)
    };
    Ok(normalize_path_lexically(&joined))
}

fn normalize_output_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        normalize_path_lexically(path)
    } else {
        normalize_path_lexically(&base_dir.join(path))
    }
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut prefix: Option<OsString> = None;
    let mut has_root = false;
    let mut parts: Vec<OsString> = Vec::new();

    for component in path.components() {
        match component {
            Component::Prefix(value) => prefix = Some(value.as_os_str().to_os_string()),
            Component::RootDir => has_root = true,
            Component::CurDir => {}
            Component::Normal(value) => parts.push(value.to_os_string()),
            Component::ParentDir => {
                if let Some(last) = parts.last()
                    && last != ".."
                {
                    parts.pop();
                    continue;
                }

                if !has_root {
                    parts.push(component.as_os_str().to_os_string());
                }
            }
        }
    }

    let mut normalized = PathBuf::new();
    if let Some(prefix) = prefix {
        normalized.push(prefix);
    }
    if has_root {
        normalized.push(std::path::MAIN_SEPARATOR.to_string());
    }
    for part in parts {
        normalized.push(part);
    }

    if normalized.as_os_str().is_empty() {
        if has_root {
            PathBuf::from(std::path::MAIN_SEPARATOR.to_string())
        } else {
            PathBuf::from(".")
        }
    } else {
        normalized
    }
}

fn normalize_phone_selector(phone: Option<&str>) -> Option<String> {
    phone
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn selected_session_phone(config: &AppConfig, phone: Option<&str>) -> Option<String> {
    normalize_phone_selector(phone)
        .or_else(|| normalize_phone_selector(config.login.phone.as_deref()))
}

fn load_fixture_session(
    fixture_dir: &std::path::Path,
    expected_phone: Option<&str>,
) -> Result<SessionRecord> {
    let record = FileSessionStore::load_path(fixture_dir.join("legacy_session.json"))?;
    if let Some(expected_phone) = expected_phone
        && record.account.phone != expected_phone
    {
        return Err(CpassError::Validation(format!(
            "session phone {expected_phone} not found in fixtures; fixture session belongs to {}",
            record.account.phone
        )));
    }
    Ok(record)
}

fn resolve_session_record(
    config: &AppConfig,
    phone: Option<&str>,
    fixture_dir: Option<&std::path::Path>,
) -> Result<SessionRecord> {
    let store = FileSessionStore::new(config.paths.session_dir.clone());
    let root = store.root().to_path_buf();
    let records = store.list()?;

    if let Some(phone) = selected_session_phone(config, phone) {
        let mut matches: Vec<SessionRecord> = records
            .into_iter()
            .filter(|record| record.account.phone == phone)
            .collect();
        return match matches.len() {
            1 => Ok(matches.remove(0)),
            0 => {
                if let Some(fixture_dir) = fixture_dir {
                    return load_fixture_session(fixture_dir, Some(&phone));
                }
                Err(CpassError::SessionNotFound(
                    root.join(format!("{phone}.json")),
                ))
            }
            _ => Err(CpassError::Validation(format!(
                "multiple saved sessions matched phone {phone} in {}; remove duplicates before running non-interactive commands",
                root.display()
            ))),
        };
    }

    match records.len() {
        0 => {
            if let Some(fixture_dir) = fixture_dir {
                load_fixture_session(fixture_dir, None)
            } else {
                Err(CpassError::SessionNotFound(root))
            }
        }
        1 => Ok(records.into_iter().next().expect("single session record")),
        _ => Err(CpassError::Validation(format!(
            "multiple saved sessions found in {}; provide --phone or set login.phone/CPASS_PHONE for non-interactive commands",
            root.display()
        ))),
    }
}

async fn client_from_session(
    config: &AppConfig,
    phone: Option<&str>,
    fixture_dir: Option<&std::path::Path>,
) -> Result<(ChaoxingClient, cpass_core::AccountProfile)> {
    let record = resolve_session_record(config, phone, fixture_dir)?;
    let transport: Arc<dyn cpass_core::ChaoxingTransport> = if let Some(fixture_dir) = fixture_dir {
        Arc::new(FixtureChaoxingTransport::new(fixture_dir.to_path_buf()))
    } else {
        Arc::new(ReqwestChaoxingTransport::new(
            config.transport.clone(),
            record.cookies.clone(),
        )?)
    };
    let client = ChaoxingClient::new(transport);
    let account = match client.fetch_account_info().await {
        Ok(account) => account,
        Err(_) => record.account,
    };
    Ok((client, account))
}

fn print_output<T: serde::Serialize>(json: bool, value: &T) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{}", serde_json::to_string(value)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::render_qr_text;

    #[test]
    fn qr_text_uses_full_block_grid_without_half_block_segments() {
        let rendered = render_qr_text("https://example.com/qr");
        assert!(rendered.contains("\x1b[40m"));
        assert!(rendered.contains("\x1b[107m"));
        assert!(!rendered.contains('▀'));
        assert!(!rendered.contains('▄'));
    }
}
