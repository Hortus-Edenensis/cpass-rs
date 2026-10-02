use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Parser, Subcommand, ValueEnum};
use cpass::{
    account,
    config::Config,
    course, exam, media,
    model::QuestionSet,
    operations,
    questions::{self, PageKind, ParsedPaper},
    resources,
    search::Searchers,
    transport::Session,
    work::Work,
    workflow,
};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    version,
    about = "超星课程、作业与考试客户端；完整且唯一才填，已有答案优先"
)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    session: Option<PathBuf>,
    #[arg(long, global = true, hide = true)]
    base_url: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 校验配置，不发网络请求
    ConfigCheck,
    /// 复用 CLI 业务的交互式终端菜单
    Tui,
    Institutions {
        #[arg(long)]
        query: String,
    },
    SmsRequest {
        #[arg(long)]
        phone: String,
        #[arg(long, default_value = "86")]
        country_code: String,
        #[arg(long)]
        captcha_env: Option<String>,
    },
    SmsLogin {
        #[arg(long)]
        phone: String,
        #[arg(long, default_value = "CPASS_SMS_CODE")]
        code_env: String,
    },
    StudentLogin {
        #[arg(long)]
        fid: u64,
        #[arg(long)]
        student_id: String,
        #[arg(long, default_value = "CPASS_PASSWORD")]
        password_env: String,
        #[arg(long)]
        captcha_env: Option<String>,
    },
    /// 顺序运行多门课程，恢复时重新核对平台状态
    RunBatch {
        #[arg(long="course-id",num_args=1..,required=true)]
        course_ids: Vec<u64>,
        #[arg(long)]
        class_id: Option<u64>,
        #[arg(long)]
        resume: Option<PathBuf>,
        #[command(flatten)]
        action: ActionArgs,
    },
    /// 提取图片、MathML 与 TeX，下载资源需要显式 --download
    Resources {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        base_url: String,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        download: Option<PathBuf>,
    },
    /// 已批阅 HTML 的只读导出
    ReviewExport {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        base_url: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// 从真实作业入口只读导出资源或已批阅答卷
    WorkExport {
        #[arg(long)]
        task_file: PathBuf,
        #[arg(long)]
        reviewed: bool,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// 主观题仅生成待人工审核材料，永不提交
    Review {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        suggest: bool,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Diagnose {
        #[arg(long)]
        log: Option<PathBuf>,
        #[arg(long)]
        output: PathBuf,
    },
    Ocr {
        #[arg(long)]
        image: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// 列出会话文件，不打印 Cookie
    Sessions,
    /// 密码从环境变量读取；--qr 使用扫码登录
    Login {
        #[arg(long, required_unless_present = "qr")]
        phone: Option<String>,
        #[arg(long, default_value = "CPASS_PASSWORD")]
        password_env: String,
        #[arg(long, conflicts_with = "phone")]
        qr: bool,
    },
    /// 导入自己已有的 Cookie，验证账户后保存
    ImportSession {
        #[arg(long, default_value = "CPASS_COOKIE")]
        cookie_env: String,
    },
    Account,
    Courses,
    Chapters {
        #[command(flatten)]
        course: CourseArgs,
    },
    Tasks {
        #[command(flatten)]
        course: CourseArgs,
        #[arg(long)]
        chapter_id: u64,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// 默认查看任务；--commit 才写平台，--final-submit 才交作业
    Run {
        #[command(flatten)]
        course: CourseArgs,
        #[arg(long)]
        chapter_id: Option<u64>,
        #[command(flatten)]
        action: ActionArgs,
    },
    Exams {
        #[command(flatten)]
        course: CourseArgs,
    },
    /// --start 会开始计时；写答案和交卷分别需要显式开关
    Exam {
        #[command(flatten)]
        course: CourseArgs,
        #[arg(long)]
        exam_id: u64,
        #[arg(long)]
        start: bool,
        #[arg(long)]
        code_env: Option<String>,
        #[arg(long)]
        captcha_env: Option<String>,
        #[arg(long)]
        face_receipt: Option<PathBuf>,
        #[command(flatten)]
        action: ActionArgs,
    },
    /// 从 tasks 导出的单个任务 JSON 读取作业
    Work {
        #[arg(long)]
        task_file: PathBuf,
        #[command(flatten)]
        action: ActionArgs,
    },
    /// 离线解析 HTML，导出兼容旧版的题目 JSON
    Parse {
        #[arg(long)]
        input: PathBuf,
        #[arg(long, value_enum)]
        kind: HtmlKind,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// 离线使用本地 JSON 题库解析答案，不调用在线搜索器
    Resolve {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        answers: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        report: Option<PathBuf>,
    },
    CaptchaImage {
        #[arg(long)]
        output: PathBuf,
    },
    CaptchaSubmit {
        #[arg(long, default_value = "CPASS_CAPTCHA")]
        code_env: String,
    },
    /// 获取手动图形验证挑战，保存挑战 JSON 供下一步使用
    ImageCaptcha {
        #[arg(long)]
        captcha_id: String,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        referer: String,
        #[arg(long)]
        output: PathBuf,
    },
    ImageCaptchaSubmit {
        #[arg(long)]
        challenge: PathBuf,
        #[arg(long)]
        coordinates: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    FaceFetch,
    FaceUpload {
        #[arg(long)]
        image: PathBuf,
    },
    CourseFace {
        #[command(flatten)]
        course: CourseArgs,
        #[arg(long)]
        chapter_id: u64,
        #[arg(long)]
        object_id: String,
    },
    ExamFace {
        #[command(flatten)]
        course: CourseArgs,
        #[arg(long)]
        exam_id: u64,
        #[arg(long)]
        object_id: String,
        #[arg(long)]
        live_status: u8,
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Args)]
struct CourseArgs {
    #[arg(long)]
    course_id: u64,
    #[arg(long)]
    class_id: Option<u64>,
}
#[derive(Args)]
struct ActionArgs {
    #[arg(long)]
    commit: bool,
    #[arg(long, requires = "commit")]
    final_submit: bool,
    #[arg(long)]
    output: Option<PathBuf>,
}
#[derive(Clone, Copy, ValueEnum)]
enum HtmlKind {
    Work,
    Exam,
}

fn env_secret(name: &str) -> Result<String> {
    let value = std::env::var(name).with_context(|| format!("环境变量 {name} 未设置"))?;
    ensure!(!value.is_empty(), "环境变量 {name} 为空");
    Ok(value)
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&std::fs::read(path).context("无法读取 JSON 文件")?)
        .context("JSON 格式不符合要求")
}
fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).context("无法创建输出临时文件")?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        Ok::<_, std::io::Error>(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.context("无法保存输出文件")
}
fn emit<T: Serialize>(value: &T, path: Option<&Path>) -> Result<()> {
    let text = serde_json::to_vec_pretty(value)?;
    if let Some(path) = path {
        write_bytes(path, &text)?;
    } else {
        println!("{}", String::from_utf8(text)?);
    }
    Ok(())
}
fn emit_report<T: Serialize>(value: &T, path: Option<&Path>) -> Result<()> {
    if path.is_some() {
        emit(value, path)
    } else {
        eprintln!("{}", serde_json::to_string_pretty(value)?);
        Ok(())
    }
}
fn select_course(session: &Session, args: &CourseArgs) -> Result<course::Course> {
    let mut courses: Vec<_> = course::list(session)?
        .into_iter()
        .filter(|c| c.id == args.course_id && args.class_id.is_none_or(|id| id == c.class_id))
        .collect();
    ensure!(
        courses.len() == 1,
        "课程不存在或有多个班级，请指定 --class-id"
    );
    Ok(courses.remove(0))
}
fn display_account(value: &account::Account, mask: bool) -> Value {
    if mask {
        json!({"puid":value.puid,"name":"***","phone":"***","school":value.school})
    } else {
        serde_json::to_value(value).unwrap()
    }
}
fn paper_result(
    paper: &ParsedPaper,
    report: &workflow::SolveReport,
    id: Value,
    kind: u8,
    path: Option<&Path>,
) -> Result<()> {
    emit(
        &json!({"questions":workflow::exported(paper,id,kind),"report":report}),
        path,
    )
}

fn run_course(
    session: &Session,
    course: &course::Course,
    uid: u64,
    config: &Config,
    chapter_id: Option<u64>,
    action: &ActionArgs,
) -> Result<Value> {
    let sources = Searchers::new(&config.searchers)?;
    let before = course::chapters(session, course, uid)?;
    ensure!(
        chapter_id.is_none_or(|id| before.iter().any(|c| c.id == id)),
        "章节不存在"
    );
    let mut outcomes = Vec::new();
    let mut task_errors = false;
    for chapter in before
        .iter()
        .filter(|c| chapter_id.is_none_or(|id| id == c.id))
    {
        if action.commit {
            course::refresh_chapter(session, course, chapter.id)?;
        }
        let tasks = match course::chapter_tasks(session, course, chapter, uid) {
            Ok(t) => t,
            Err(e) => {
                task_errors = true;
                outcomes
                    .push(json!({"chapter":chapter.id,"status":"未完成","reason":e.to_string()}));
                continue;
            }
        };
        for task in tasks {
            let outcome = (|| -> Result<Value> {
                if task.kind == course::TaskKind::Unknown {
                    bail!("不支持的任务类型，保持未完成");
                }
                if task.already_complete() {
                    return Ok(json!({"status":"已有完成状态"}));
                }
                match task.kind {
                    course::TaskKind::Work if config.work.enable || config.work.export => {
                        let mut work = Work::from_task(session.clone(), &task, uid)?;
                        let (paper, report) = workflow::run_work(
                            &mut work,
                            &sources,
                            action.commit && config.work.enable,
                            action.final_submit && config.work.enable,
                            config.work.fallback_save,
                        )?;
                        task_errors |= !report.complete();
                        record_resolution(config, operations::Stage::Work, &report);
                        if config.work.export {
                            emit(
                                &workflow::exported(&paper, task.property["workid"].clone(), 1),
                                Some(&config.export_path.join(format!(
                                        "work-{}.json",
                                        task.property["workid"]
                                            .as_str()
                                            .filter(|id| id
                                                .chars()
                                                .all(|c| c.is_ascii_alphanumeric()
                                                    || c == '-'
                                                    || c == '_'))
                                            .unwrap_or("unknown")
                                    ))),
                            )?;
                        }
                        Ok(
                            json!({"report":report,"questions":workflow::exported(&paper,task.property["workid"].clone(),1)}),
                        )
                    }
                    course::TaskKind::Video if action.commit && config.video.enable => {
                        media::run_video(
                            session,
                            &task,
                            uid,
                            config.video.speed,
                            config.video.report_rate,
                        )
                    }
                    course::TaskKind::Document if action.commit && config.document.enable => {
                        media::run_document(session, &task)
                    }
                    course::TaskKind::Live => media::run_live(session, &task),
                    course::TaskKind::Article => media::run_article(session, &task),
                    _ => Ok(json!({"status":"未执行","reason":"任务写入未启用"})),
                }
            })();
            let failed = match &outcome {
                Ok(value) => value["action_required"] == true,
                Err(_) => true,
            };
            task_errors |= failed;
            outcomes.push(json!({"chapter":chapter.id,"card":task.card_index,"kind":task.kind,"result":outcome.unwrap_or_else(|e|json!({"status":"未完成","reason":e.to_string()}))}));
            if action.commit && !failed {
                let wait = match task.kind {
                    course::TaskKind::Work => config.work.wait,
                    course::TaskKind::Video => config.video.wait,
                    _ => config.document.wait,
                };
                if wait > 0 {
                    std::thread::sleep(Duration::from_secs(wait));
                }
            }
        }
    }
    let after = course::chapters(session, course, uid)?;
    Ok(
        json!({"course":course,"tasks":outcomes,"chapters_before":before,"chapters_after":after,"failed":task_errors,"platform_completion_verified":!task_errors&&after.iter().map(|c|c.total).sum::<usize>()>0&&after.iter().all(|c|c.finished==c.total)}),
    )
}

fn run(cli: Cli) -> Result<()> {
    match &cli.command {
        Command::Resources {
            input,
            base_url,
            output,
            download,
        } => {
            let mut manifest = resources::extract(&std::fs::read_to_string(input)?, base_url)?;
            if let Some(directory) = download {
                resources::download(&mut manifest, directory)?;
            }
            emit(&manifest, output.as_deref())?;
            return Ok(());
        }
        Command::ReviewExport {
            input,
            base_url,
            output,
        } => {
            let review = resources::parse_review(&std::fs::read_to_string(input)?, base_url)?;
            emit(&review, output.as_deref())?;
            ensure!(
                review.parse_errors.is_empty(),
                "批阅部分字段无法验证，结果已保留"
            );
            return Ok(());
        }
        Command::Parse {
            input,
            kind,
            output,
            report,
        } => {
            let page_kind = match kind {
                HtmlKind::Work => PageKind::Work,
                HtmlKind::Exam => PageKind::Exam,
            };
            let paper = questions::parse_page(&std::fs::read_to_string(input)?, page_kind)?;
            emit(
                &workflow::exported(
                    &paper,
                    Value::Null,
                    match kind {
                        HtmlKind::Work => 1,
                        HtmlKind::Exam => 0,
                    },
                ),
                output.as_deref(),
            )?;
            emit_report(
                &json!({"parsed":paper.questions.len(),"declared_count":paper.declared_count,"parse_errors":paper.parse_errors}),
                report.as_deref(),
            )?;
            ensure!(
                paper.parse_errors.is_empty(),
                "部分题目解析失败，结果已导出"
            );
            return Ok(());
        }
        Command::Resolve {
            input,
            answers,
            output,
            report,
        } => {
            let mut set: QuestionSet = read_json(input)?;
            let searchers =
                Searchers::new(&[json!({"type":"jsonFileSearcher","file_path":answers})])?;
            let mut paper = ParsedPaper {
                indices: (0..set.questions.len()).collect(),
                declared_count: Some(set.questions.len()),
                questions: std::mem::take(&mut set.questions),
                parse_errors: BTreeMap::new(),
                form: BTreeMap::new(),
                title: set.title.clone(),
            };
            let resolution = workflow::resolve_paper(&mut paper, &searchers);
            set.questions = paper.questions;
            emit(&set, output.as_deref())?;
            emit_report(&resolution, report.as_deref())?;
            ensure!(resolution.complete(), "存在未匹配／未完成题目，结果已导出");
            return Ok(());
        }
        _ => {}
    }
    let config_path = cli.config.clone();
    let config = match cli.config {
        Some(path) => Config::load(&path)?,
        None if Path::new("config.yml").exists() => Config::load(Path::new("config.yml"))?,
        None => Config::default(),
    };
    config.validate()?;
    let session_file = cli
        .session
        .unwrap_or_else(|| config.session_path.join("default.json"));
    if let Command::ConfigCheck = cli.command {
        Searchers::new(&config.searchers)?.validate()?;
        println!(
            "配置有效；{} 个搜索源；路径相对配置文件解析",
            config.searchers.len()
        );
        return Ok(());
    }
    if let Command::Sessions = cli.command {
        let files: Vec<_> = if config.session_path.exists() {
            std::fs::read_dir(&config.session_path)?
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|v| v == "json"))
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        } else {
            vec![]
        };
        emit(&files, None)?;
        return Ok(());
    }
    match &cli.command {
        Command::Tui => return terminal_menu(config_path, Some(session_file), cli.base_url),
        Command::Diagnose { log, output } => {
            operations::diagnostic_bundle(
                log.as_deref()
                    .unwrap_or(&config.log_path.join("events.jsonl")),
                output,
            )?;
            return Ok(());
        }
        Command::Ocr { image, output } => {
            emit(
                &operations::ocr_hint(&config.ocr, image)?,
                output.as_deref(),
            )?;
            return Ok(());
        }
        Command::Review {
            input,
            suggest,
            output,
        } => {
            let set: QuestionSet = read_json(input)?;
            let sources = if *suggest {
                Some(Searchers::new(&config.searchers)?)
            } else {
                None
            };
            let questions:Vec<_>=set.questions.iter().filter(|q|matches!(q.kind.0,4|5|6|7|9|10)).map(|q|{
                let drafts=sources.as_ref().map(|sources|sources.search(q)).unwrap_or_default();
                json!({"question":q,"drafts":drafts,"requires_human_review":true,"automatic_submission":false})
            }).collect();
            emit(
                &json!({"id":set.id,"title":set.title,"questions":questions,"requires_human_review":true}),
                output.as_deref(),
            )?;
            return Ok(());
        }
        _ => {}
    }
    if config.work.fallback_fuzzer || config.exam.fallback_fuzzer {
        eprintln!("严格策略已忽略 fallback_fuzzer；不会随机填答");
    }
    let session = Session::new(
        config.timeout_secs,
        config.request_retries,
        cli.base_url.as_deref(),
    )?;
    if session_file.exists() {
        session.load(&session_file)?;
    }
    match cli.command {
        Command::Institutions { query } => emit(&account::institutions(&session, &query)?, None)?,
        Command::SmsRequest {
            phone,
            country_code,
            captcha_env,
        } => {
            let validate = captcha_env.as_deref().map(env_secret).transpose()?;
            account::request_sms(&session, &phone, &country_code, validate.as_deref())?;
            session.save(&session_file)?;
            println!("短信发送请求已接受，请手工读取验证码");
        }
        Command::SmsLogin { phone, code_env } => {
            let account = account::login_sms(&session, &phone, &env_secret(&code_env)?)?;
            session.save(&session_file)?;
            emit(&display_account(&account, config.mask_acc), None)?;
        }
        Command::StudentLogin {
            fid,
            student_id,
            password_env,
            captcha_env,
        } => {
            let validate = captcha_env.as_deref().map(env_secret).transpose()?;
            let account = account::login_student(
                &session,
                fid,
                &student_id,
                &env_secret(&password_env)?,
                validate.as_deref(),
            )?;
            session.save(&session_file)?;
            emit(&display_account(&account, config.mask_acc), None)?;
        }
        Command::Login {
            phone,
            password_env,
            qr,
        } => {
            let account = if qr {
                let challenge = account::qr_begin(&session)?;
                let code = qrcode::QrCode::new(challenge.url.as_bytes())?;
                println!(
                    "{}",
                    code.render::<char>()
                        .quiet_zone(true)
                        .module_dimensions(2, 1)
                        .dark_color('█')
                        .light_color(' ')
                        .build()
                );
                let deadline = Instant::now() + Duration::from_secs(120);
                loop {
                    if let Some(account) = account::qr_poll(&session, &challenge)? {
                        break account;
                    }
                    ensure!(Instant::now() < deadline, "扫码登录超时，请重试");
                    std::thread::sleep(Duration::from_secs(2));
                }
            } else {
                account::login_password(
                    &session,
                    phone.as_deref().context("缺少手机号")?,
                    &env_secret(&password_env)?,
                )?
            };
            session.save(&session_file)?;
            emit(&display_account(&account, config.mask_acc), None)?;
        }
        Command::ImportSession { cookie_env } => {
            session.import_cookie(&env_secret(&cookie_env)?)?;
            let account = account::account(&session)?;
            session.save(&session_file)?;
            emit(&display_account(&account, config.mask_acc), None)?;
        }
        Command::CaptchaImage { output } => {
            write_bytes(&output, &account::fetch_captcha(&session)?)?
        }
        Command::CaptchaSubmit { code_env } => {
            account::solve_captcha(&session, &env_secret(&code_env)?)?;
            session.save(&session_file)?;
            println!("验证码回执成功");
        }
        Command::ImageCaptcha {
            captcha_id,
            kind,
            referer,
            output,
        } => emit(
            &account::image_captcha_begin(&session, &captcha_id, &kind, &referer)?,
            Some(&output),
        )?,
        Command::ImageCaptchaSubmit {
            challenge,
            coordinates,
            output,
        } => {
            let result = account::image_captcha_submit(
                &session,
                &read_json(&challenge)?,
                &read_json(&coordinates)?,
            )?;
            emit(&json!({"validate":result}), Some(&output))?;
        }
        command => {
            let account = account::account(&session)?;
            let uid = account.puid;
            match command {
                Command::Account => emit(&display_account(&account, config.mask_acc), None)?,
                Command::Courses => emit(&course::list(&session)?, None)?,
                Command::Chapters { course: args } => emit(
                    &course::chapters(&session, &select_course(&session, &args)?, uid)?,
                    None,
                )?,
                Command::Tasks {
                    course: args,
                    chapter_id,
                    output,
                } => {
                    let course = select_course(&session, &args)?;
                    let chapter = course::chapters(&session, &course, uid)?
                        .into_iter()
                        .find(|c| c.id == chapter_id)
                        .context("章节不存在")?;
                    emit(
                        &course::chapter_tasks(&session, &course, &chapter, uid)?,
                        output.as_deref(),
                    )?;
                }
                Command::Exams { course: args } => emit(
                    &exam::list(&session, &select_course(&session, &args)?)?,
                    None,
                )?,
                Command::Work { task_file, action } => {
                    let task: course::Task = read_json(&task_file)?;
                    ensure!(task.kind == course::TaskKind::Work, "任务类型必须是 Work");
                    let mut work = Work::from_task(session.clone(), &task, uid)?;
                    let sources = Searchers::new(&config.searchers)?;
                    let (paper, report) = workflow::run_work(
                        &mut work,
                        &sources,
                        action.commit,
                        action.final_submit,
                        config.work.fallback_save,
                    )?;
                    record_resolution(&config, operations::Stage::Work, &report);
                    paper_result(
                        &paper,
                        &report,
                        task.property["workid"].clone(),
                        1,
                        action.output.as_deref(),
                    )?;
                    session.save(&session_file)?;
                    ensure!(report.complete(), "作业有未完成或提交失败，未自动交卷");
                }
                Command::Exam {
                    course: args,
                    exam_id,
                    start,
                    code_env,
                    captcha_env,
                    face_receipt,
                    action,
                } => {
                    ensure!(start || !action.commit, "写入考试必须同时指定 --start");
                    let course = select_course(&session, &args)?;
                    let info = exam::list(&session, &course)?
                        .into_iter()
                        .find(|e| e.id == exam_id)
                        .context("考试不存在")?;
                    let mut exam = exam::Exam::new(session.clone(), info, uid);
                    let face: Option<Value> = face_receipt.as_deref().map(read_json).transpose()?;
                    exam.set_challenge_data(
                        captcha_env.as_deref().map(env_secret).transpose()?,
                        face.as_ref()
                            .and_then(|v| v["facekey"].as_str().map(str::to_owned)),
                        face.clone(),
                    );
                    let metadata = exam.metadata()?;
                    if !start {
                        emit(&metadata, action.output.as_deref())?;
                    } else {
                        let code = code_env.as_deref().map(env_secret).transpose()?;
                        exam.start(code.as_deref())?;
                        let (paper, report) = workflow::run_exam(
                            &mut exam,
                            &Searchers::new(&config.searchers)?,
                            action.commit,
                            action.final_submit,
                            config.exam.persubmit_delay,
                        )?;
                        record_resolution(&config, operations::Stage::Exam, &report);
                        paper_result(&paper, &report, json!(exam_id), 0, action.output.as_deref())?;
                        session.save(&session_file)?;
                        ensure!(report.complete(), "考试有未完成或提交失败，未自动交卷");
                    }
                }
                Command::Run {
                    course: args,
                    chapter_id,
                    action,
                } => {
                    let report = run_course(
                        &session,
                        &select_course(&session, &args)?,
                        uid,
                        &config,
                        chapter_id,
                        &action,
                    )?;
                    emit(&report, action.output.as_deref())?;
                    session.save(&session_file)?;
                    ensure!(
                        report["failed"] != true,
                        "有任务未完成或提交失败，详细报告已保存"
                    );
                }
                Command::RunBatch {
                    course_ids,
                    class_id,
                    resume,
                    action,
                } => {
                    let mut ids = std::collections::BTreeSet::new();
                    ensure!(
                        !course_ids.is_empty()
                            && course_ids.iter().all(|id| *id > 0 && ids.insert(*id)),
                        "课程 ID 无效、重复或未提供"
                    );
                    ensure!(
                        match (&resume, &action.output) {
                            (Some(checkpoint), Some(output)) =>
                                resolved_path(checkpoint)? != resolved_path(output)?,
                            _ => true,
                        },
                        "checkpoint 与 output 必须为不同文件"
                    );
                    if let Some(path) = resume.as_ref().filter(|p| p.exists()) {
                        let checkpoint: Value = read_json(path)?;
                        ensure!(
                            checkpoint["version"] == 1
                                && checkpoint["uid"] == uid
                                && checkpoint["course_ids"] == json!(course_ids)
                                && checkpoint["class_id"] == json!(class_id),
                            "checkpoint 与当前账户或课程不一致"
                        );
                    }
                    let mut reports = Vec::new();
                    let mut failed = false;
                    for id in &course_ids {
                        let result = (|| -> Result<Value> {
                            let course = select_course(
                                &session,
                                &CourseArgs {
                                    course_id: *id,
                                    class_id,
                                },
                            )?;
                            if resume.is_some() {
                                let chapters = course::chapters(&session, &course, uid)?;
                                if chapters.iter().map(|c| c.total).sum::<usize>() > 0
                                    && chapters.iter().all(|c| c.finished == c.total)
                                {
                                    return Ok(
                                        json!({"course":course,"skipped":true,"failed":false,"platform_completion_verified":true}),
                                    );
                                }
                            }
                            run_course(&session, &course, uid, &config, None, &action)
                        })();
                        let report = result.unwrap_or_else(
                            |e| json!({"course_id":id,"failed":true,"reason":e.to_string()}),
                        );
                        failed |= report["failed"] == true;
                        reports.push(report);
                        if let Some(path) = &resume {
                            emit(
                                &json!({"version":1,"uid":uid,"course_ids":course_ids,"class_id":class_id,"reports":reports}),
                                Some(path),
                            )?;
                        }
                        session.save(&session_file)?;
                    }
                    emit(
                        &json!({"reports":reports,"failed":failed}),
                        action.output.as_deref(),
                    )?;
                    ensure!(!failed, "批量任务有未完成或失败，checkpoint 与报告已保留");
                }
                Command::WorkExport {
                    task_file,
                    reviewed,
                    output,
                } => {
                    let task: course::Task = read_json(&task_file)?;
                    ensure!(task.kind == course::TaskKind::Work, "必须指定 Work 任务");
                    let work = Work::from_task(session.clone(), &task, uid)?;
                    if reviewed {
                        let review = work.fetch_review()?;
                        emit(&review, output.as_deref())?;
                        ensure!(review.parse_errors.is_empty(), "批阅字段不完整，已保留结果");
                    } else {
                        emit(&work.fetch_resources()?, output.as_deref())?;
                    }
                }
                Command::FaceFetch => {
                    emit(&json!({"url":account::fetch_face(&session,uid)?}), None)?
                }
                Command::FaceUpload { image } => emit(
                    &json!({"object_id":account::upload_face(&session,uid,&image)?}),
                    None,
                )?,
                Command::CourseFace {
                    course: args,
                    chapter_id,
                    object_id,
                } => {
                    let course = select_course(&session, &args)?;
                    emit(
                        &account::verify_course_face(
                            &session,
                            course.id,
                            course.class_id,
                            chapter_id,
                            course.cpi,
                            &object_id,
                        )?,
                        None,
                    )?;
                }
                Command::ExamFace {
                    course: args,
                    exam_id,
                    object_id,
                    live_status,
                    output,
                } => {
                    let course = select_course(&session, &args)?;
                    emit(
                        &account::compare_exam_face(
                            &session,
                            exam_id,
                            course.id,
                            course.class_id,
                            course.cpi,
                            &object_id,
                            Some(live_status),
                        )?,
                        Some(&output),
                    )?;
                }
                _ => unreachable!(),
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run_and_record(Cli::parse()) {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn resolved_path(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut existing = absolute.as_path();
    let mut suffix = Vec::new();
    while !existing.exists() {
        suffix.push(existing.file_name().context("路径无效")?.to_owned());
        existing = existing.parent().context("路径无效")?;
    }
    let mut resolved = existing.canonicalize()?;
    for part in suffix.into_iter().rev() {
        resolved.push(part);
    }
    #[cfg(windows)]
    {
        // Case aliases must be rejected before either output exists.
        Ok(PathBuf::from(resolved.to_string_lossy().to_lowercase()))
    }
    #[cfg(not(windows))]
    {
        Ok(resolved)
    }
}

fn event_log(config: &Config, event: &operations::Event, notify: bool) {
    if let Err(_error) = (|| -> Result<()> {
        std::fs::create_dir_all(&config.log_path)?;
        let log = config.log_path.join("events.jsonl");
        if log.exists() && std::fs::metadata(&log)?.len() > 3 * 1024 * 1024 {
            let archive = config
                .log_path
                .join(format!("events-{}.jsonl", cpass::transport::timestamp()));
            std::fs::rename(&log, archive)?;
        }
        let cutoff =
            cpass::transport::timestamp().saturating_sub(config.log_retention_days * 86400000);
        for entry in std::fs::read_dir(&config.log_path)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(time) = name
                .strip_prefix("events-")
                .and_then(|s| s.strip_suffix(".jsonl"))
                .and_then(|s| s.parse::<u64>().ok())
                && time < cutoff
                && entry.file_type()?.is_file()
            {
                std::fs::remove_file(entry.path())?;
            }
        }
        operations::append_log(&log, event)?;
        Ok(())
    })() {
        eprintln!("结构化日志写入失败，业务结果已保留");
    }
    for receipt in if notify {
        operations::notify(&config.notifications, event)
    } else {
        Vec::new()
    } {
        if !receipt.accepted {
            eprintln!("{} 通知未确认接受", receipt.provider);
        }
    }
}

fn record_resolution(config: &Config, stage: operations::Stage, report: &workflow::SolveReport) {
    let mut event = operations::Event::new(
        stage,
        if report.complete() {
            operations::Outcome::Succeeded
        } else {
            operations::Outcome::Incomplete
        },
        if report.complete() {
            None
        } else {
            Some(operations::ErrorCode::Incomplete)
        },
    );
    event.matched = report.matched;
    event.submitted = report.submitted;
    event.incomplete = report.incomplete;
    event_log(config, &event, true);
}

fn run_and_record(cli: Cli) -> Result<()> {
    let notify = !matches!(
        cli.command,
        Command::Parse { .. }
            | Command::Resolve { .. }
            | Command::Review { .. }
            | Command::ReviewExport { .. }
            | Command::Resources { .. }
            | Command::Ocr { .. }
    );
    let should_record = !matches!(
        cli.command,
        Command::Tui | Command::ConfigCheck | Command::Sessions | Command::Diagnose { .. }
    );
    let config_path = cli.config.clone().or_else(|| {
        Path::new("config.yml")
            .exists()
            .then(|| PathBuf::from("config.yml"))
    });
    let result = run(cli);
    if should_record
        && let Some(path) = config_path
        && let Ok(config) = Config::load(&path)
    {
        let event = operations::Event::new(
            operations::Stage::Command,
            if result.is_ok() {
                operations::Outcome::Succeeded
            } else {
                operations::Outcome::Failed
            },
            if result.is_ok() {
                None
            } else {
                Some(operations::ErrorCode::Operation)
            },
        );
        event_log(&config, &event, notify);
    }
    result
}

fn prompt(label: &str) -> Result<String> {
    use std::io::Write;
    eprint!("{label}: ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    ensure!(std::io::stdin().read_line(&mut line)? > 0, "输入已结束");
    Ok(line.trim().to_owned())
}

fn terminal_menu(
    config: Option<PathBuf>,
    session: Option<PathBuf>,
    base_url: Option<String>,
) -> Result<()> {
    loop {
        eprintln!(
            "\ncpass Rust\n1 账户  2 课程  3 章节  4 查看任务\n5 执行课程  6 考试列表  7 扫码登录  8 密码登录\n0 退出"
        );
        let mut choice = String::new();
        if std::io::stdin().read_line(&mut choice)? == 0 || choice.trim() == "0" {
            break;
        }
        let command = (|| -> Result<Command> {
            let course = || -> Result<CourseArgs> {
                let course_id = prompt("课程ID")?.parse().context("课程ID无效")?;
                let class = prompt("班级ID（可留空）")?;
                Ok(CourseArgs {
                    course_id,
                    class_id: if class.is_empty() {
                        None
                    } else {
                        Some(class.parse().context("班级ID无效")?)
                    },
                })
            };
            Ok(match choice.trim() {
                "1" => Command::Account,
                "2" => Command::Courses,
                "3" => Command::Chapters { course: course()? },
                "4" => Command::Tasks {
                    course: course()?,
                    chapter_id: prompt("章节ID")?.parse()?,
                    output: None,
                },
                "5" => {
                    let course = course()?;
                    let commit = prompt("写入平台请输入 yes，其他输入仅查看")? == "yes";
                    let final_submit =
                        commit && prompt("最终交作业请输入 yes，其他输入仅保存")? == "yes";
                    Command::Run {
                        course,
                        chapter_id: None,
                        action: ActionArgs {
                            commit,
                            final_submit,
                            output: None,
                        },
                    }
                }
                "6" => Command::Exams { course: course()? },
                "7" => Command::Login {
                    phone: None,
                    password_env: "CPASS_PASSWORD".into(),
                    qr: true,
                },
                "8" => Command::Login {
                    phone: Some(prompt("手机号")?),
                    password_env: "CPASS_PASSWORD".into(),
                    qr: false,
                },
                _ => bail!("菜单选项无效"),
            })
        })();
        match command.and_then(|command| {
            run_and_record(Cli {
                config: config.clone(),
                session: session.clone(),
                base_url: base_url.clone(),
                command,
            })
        }) {
            Ok(()) => eprintln!("操作完成"),
            Err(error) => eprintln!("{error:#}"),
        }
    }
    Ok(())
}
