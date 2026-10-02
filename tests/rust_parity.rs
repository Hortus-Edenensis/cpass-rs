use cpass::model::{Question, QuestionSet, QuestionType};
use cpass::questions::{
    PageKind, exam_form, fill, normalize_answer, parse_page, valid_answer, work_form,
};
use serde_json::{Value, json};

const WORK: &str = include_str!("fixtures/work_questions.html");
const EXAM: &str = include_str!("fixtures/exam_questions.html");

fn question(kind: QuestionType, options: Value, answer: Value) -> Question {
    Question {
        id: 42,
        value: "以下不正确的是？".into(),
        kind,
        options,
        answer,
    }
}

fn single() -> Question {
    question(
        QuestionType::SINGLE,
        json!({"A":"甲", "B":"乙", "C":"丙"}),
        Value::Null,
    )
}

#[test]
fn work_and_exam_preserve_inline_paragraphs_options_and_saved_false() {
    for (html, kind) in [(WORK, PageKind::Work), (EXAM, PageKind::Exam)] {
        for html in [html.to_string(), html.replace('\n', "")] {
            let paper = parse_page(&html, kind).unwrap();
            assert!(paper.parse_errors.is_empty(), "{:?}", paper.parse_errors);
            assert_eq!(paper.indices, [0, 1, 2, 3]);
            assert_eq!(paper.questions.len(), 4);
            assert_eq!(paper.questions[0].kind, QuestionType::SINGLE);
            assert_eq!(paper.questions[0].value, "第一行\n第二行\n第三行");
            assert_eq!(
                paper.questions[0].options,
                json!({"A":"甲选项", "B":"乙选项"})
            );
            assert_eq!(paper.questions[1].kind, QuestionType::MULTIPLE);
            assert_eq!(paper.questions[2].answer, json!(["  已有  ", ""]));
            assert_eq!(paper.questions[3].answer, json!(false));
            assert!(valid_answer(&paper.questions[3]));
        }
    }
}

#[test]
fn numeric_type_is_authoritative_and_only_missing_or_non_numeric_uses_title() {
    for (html, kind, field, title) in [
        (
            WORK,
            PageKind::Work,
            "id=\"answertype42\" value=\"0\"",
            "单选题",
        ),
        (
            EXAM,
            PageKind::Exam,
            "name=\"type42\" value=\"0\"",
            "判断题",
        ),
    ] {
        for (raw, expected) in [("12", 12), ("999", 999), ("4", 4)] {
            let changed = html.replacen(
                field,
                &field.replace("value=\"0\"", &format!("value=\"{raw}\"")),
                1,
            );
            let paper = parse_page(&changed, kind).unwrap();
            assert_eq!(paper.questions[0].kind.0, expected);
            assert!(!paper.questions[0].kind.supported());
            assert!(!valid_answer(&paper.questions[0]));
        }
        let changed = html.replacen(field, &field.replace("value=\"0\"", "value=\"invalid\""), 1);
        let paper = parse_page(&changed, kind).unwrap();
        assert_eq!(
            paper.questions[0].kind,
            if title == "单选题" {
                QuestionType::SINGLE
            } else {
                QuestionType::TRUE_FALSE
            }
        );
    }
}

#[test]
fn malformed_and_duplicate_ids_are_isolated_and_count_mismatch_is_recorded() {
    let malformed = WORK
        .replace("data=\"42\"", "data=\"\"")
        .replace("id=\"answertype42\"", "id=\"untrusted\"");
    let paper = parse_page(&malformed, PageKind::Work).unwrap();
    assert!(paper.parse_errors.contains_key(&0));
    assert_eq!(paper.indices, [1, 2, 3]);
    assert_eq!(
        paper.questions.iter().map(|q| q.id).collect::<Vec<_>>(),
        [43, 44, 45]
    );
    let missing = parse_page(
        &WORK.replace(
            "id=\"totalQuestionNum\" value=\"4\"",
            "id=\"totalQuestionNum\" value=\"5\"",
        ),
        PageKind::Work,
    )
    .unwrap();
    assert!(!missing.parse_errors.is_empty());
    for (html, kind) in [(WORK, PageKind::Work), (EXAM, PageKind::Exam)] {
        let duplicate = html.replace("43", "42");
        let paper = parse_page(&duplicate, kind).unwrap();
        assert!(!paper.parse_errors.is_empty());
        let mut ids = paper.questions.iter().map(|q| q.id).collect::<Vec<_>>();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }
}

#[test]
fn single_choice_requires_unique_exact_content_or_consistent_label() {
    let q = single();
    for raw in [
        json!("A"),
        json!("Ａ"),
        json!("A. 甲"),
        json!("甲"),
        json!(["甲"]),
    ] {
        assert_eq!(normalize_answer(&q, &raw), Some(json!("A")), "{raw}");
    }
    for raw in [json!("A. 乙"), json!("甲的解释"), json!("D"), json!(null)] {
        assert_eq!(normalize_answer(&q, &raw), None, "{raw}");
    }
    let ambiguous = question(
        QuestionType::SINGLE,
        json!({"A":"甲", "B":"甲"}),
        Value::Null,
    );
    assert_eq!(normalize_answer(&ambiguous, &json!("甲")), None);
    assert_eq!(normalize_answer(&ambiguous, &json!("A")), Some(json!("A")));
}

#[test]
fn math_case_punctuation_and_interval_text_keep_their_meaning() {
    let q = question(
        QuestionType::SINGLE,
        json!({"A":"x < 1", "B":"x ≤ 1", "C":"X < 1", "D":"x < 1!"}),
        Value::Null,
    );
    for (raw, expected) in [
        ("x &lt; 1", "A"),
        ("x ≤ 1", "B"),
        ("X < 1", "C"),
        ("x < 1!", "D"),
    ] {
        assert_eq!(normalize_answer(&q, &json!(raw)), Some(json!(expected)));
    }
    assert_eq!(normalize_answer(&q, &json!("x < 1.")), None);
    let interval = question(
        QuestionType::SINGLE,
        json!({"A":"[0,1]", "B":"(0,1)"}),
        Value::Null,
    );
    assert_eq!(
        normalize_answer(&interval, &json!("[0,1]")),
        Some(json!("A"))
    );
    assert_eq!(
        normalize_answer(&interval, &json!("(0,1)")),
        Some(json!("B"))
    );
}

#[test]
fn multiple_choice_never_accepts_partial_matches_and_retains_option_order() {
    let q = question(
        QuestionType::MULTIPLE,
        json!({"C":"丙", "A":"甲", "B":"乙"}),
        Value::Null,
    );
    for raw in [
        json!(["甲", "丙", "甲"]),
        json!("A;C;A"),
        json!("AC"),
        json!("甲#丙"),
    ] {
        assert_eq!(normalize_answer(&q, &raw), Some(json!("CA")), "{raw}");
    }
    for raw in [json!(["甲", "不存在"]), json!("AD"), json!(""), json!([])] {
        assert_eq!(normalize_answer(&q, &raw), None, "{raw}");
    }
}

#[test]
fn judgment_false_is_an_answer_and_substring_guesses_are_rejected() {
    let q = question(QuestionType::TRUE_FALSE, Value::Null, Value::Null);
    for raw in [
        json!(false),
        json!("FALSE"),
        json!("不正确"),
        json!("错误"),
        json!("×"),
        json!(0),
    ] {
        assert_eq!(normalize_answer(&q, &raw), Some(json!(false)), "{raw}");
    }
    for raw in [
        json!(true),
        json!("true"),
        json!("正确"),
        json!("√"),
        json!(1),
    ] {
        assert_eq!(normalize_answer(&q, &raw), Some(json!(true)), "{raw}");
    }
    for raw in [
        json!("不一定正确"),
        json!("此说法正确因为..."),
        json!(null),
        json!(""),
        json!(0.0),
        json!(1.0),
    ] {
        assert_eq!(normalize_answer(&q, &raw), None, "{raw}");
    }
    let mut saved = question(QuestionType::TRUE_FALSE, Value::Null, json!(false));
    assert!(valid_answer(&saved));
    assert!(fill(&mut saved, &[json!(true)]).unwrap());
    assert_eq!(saved.answer, json!(false));
}

#[test]
fn partial_blanks_require_exact_count_and_preserve_original_existing_values() {
    let original = question(
        QuestionType::BLANK,
        json!(["一", "二"]),
        json!(["  已有  ", ""]),
    );
    for raw in [
        json!(["已有"]),
        json!(["已有", ""]),
        json!(["已有", "新", "多余"]),
        json!(["冲突", "新"]),
    ] {
        let mut q = original.clone();
        assert!(!fill(&mut q, &[raw]).unwrap());
        assert_eq!(q.answer, original.answer);
    }
    let mut q = original;
    assert!(fill(&mut q, &[json!(["已有", "新"])]).unwrap());
    assert_eq!(q.answer, json!(["  已有  ", "新"]));
}

#[test]
fn explicit_answers_are_distinct_from_analysis_in_all_supported_wrappers() {
    let q = single();
    assert_eq!(
        normalize_answer(&q, &json!("{\"answer\":\"A\"}")),
        Some(json!("A"))
    );
    assert_eq!(
        normalize_answer(&q, &json!("答案：A\n解析：说明")),
        Some(json!("A"))
    );
    assert_eq!(
        normalize_answer(&q, &json!("分析 A 正确，因此选择 A")),
        None
    );
    let blank = question(QuestionType::BLANK, json!(["一"]), Value::Null);
    for raw in [
        json!("答案：北京，解析：北京是首都"),
        json!(["北京，解析：北京是首都"]),
        json!("[\"北京，解析：北京是首都\"]"),
        json!("{\"answer\":[\"北京，解析：北京是首都\"]}"),
        json!(["北京，解\u{200b}析：北京是首都"]),
    ] {
        assert_eq!(normalize_answer(&blank, &raw), None, "{raw}");
    }
    assert_eq!(
        normalize_answer(&blank, &json!("答案：北京\n解析：北京是首都")),
        Some(json!(["北京"]))
    );
}

#[test]
fn equivalent_sources_merge_but_conflicts_leave_answer_untouched() {
    let mut q = single();
    assert!(fill(&mut q, &[json!("A"), json!("甲"), json!("Ａ. 甲")]).unwrap());
    assert_eq!(q.answer, json!("A"));
    let mut q = single();
    assert!(fill(&mut q, &[json!("A"), json!("B")]).is_err());
    assert!(q.answer.is_null());
    assert!(!fill(&mut q, &[]).unwrap());
    assert!(q.answer.is_null());
}

#[test]
fn forms_filter_invalid_values_and_encode_false_only_for_real_boolean() {
    let invalid = [
        question(QuestionType::TRUE_FALSE, Value::Null, Value::Null),
        question(QuestionType::TRUE_FALSE, Value::Null, json!("false")),
        question(QuestionType::BLANK, json!(["一", "二"]), json!(["甲"])),
        question(QuestionType::SINGLE, json!({"A":"甲"}), json!("Z")),
    ];
    for q in invalid {
        assert!(!valid_answer(&q));
        assert!(exam_form(&q).is_err());
        let form = work_form(&[q]);
        assert!(!form.contains_key("answer42"));
        assert!(!form.contains_key("answer421"));
        assert!(!form.contains_key("tiankongsize42"));
    }
    let q = question(QuestionType::TRUE_FALSE, Value::Null, json!(false));
    assert_eq!(exam_form(&q).unwrap().get("answer42").unwrap(), "false");
    assert_eq!(work_form(&[q]).get("answer42").unwrap(), "false");
}

#[test]
fn legacy_export_shape_and_numeric_type_round_trip_without_losing_false_or_null() {
    let legacy = json!({"id":"work-7", "title":"离线试卷", "type":1, "questions":[
        {"id":42,"value":"判断", "type":3,"options":null,"answer":false},
        {"id":43,"value":"填空", "type":2,"options":["一","二"],"answer":["甲",null]},
        {"id":44,"value":"未知", "type":999,"options":null,"answer":null}
    ]});
    let export: QuestionSet = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(export).unwrap(), legacy);
}

#[test]
fn json_search_is_exact_preserves_false_and_detects_duplicate_key_conflicts() {
    use cpass::search::Searchers;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/answers.json");
    let searchers =
        Searchers::new(&[json!({"type":"JsonFileSearcher", "file_path":path})]).unwrap();
    let mut q = single();
    for (title, expected) in [("以下正确的是？", "A"), ("以下不正确的是？", "B")] {
        q.value = title.into();
        let candidates = searchers.search(&q);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].answer, json!(expected));
        assert!(candidates[0].error.is_none());
    }
    q.value = "以下正确的是".into();
    assert!(searchers.search(&q).iter().all(|c| c.error.is_some()));
    q.value = "重复冲突".into();
    assert!(searchers.search(&q).iter().any(|c| c.conflict));
    q.value = "等价题".into();
    let candidates = searchers.search(&q);
    assert!(candidates.iter().all(|c| !c.conflict && c.error.is_none()));
    assert!(
        fill(
            &mut q,
            &candidates.into_iter().map(|c| c.answer).collect::<Vec<_>>()
        )
        .unwrap()
    );
    assert_eq!(q.answer, json!("A"));
    q = question(QuestionType::TRUE_FALSE, Value::Null, Value::Null);
    q.value = "判断\u{a0}题".into();
    let candidates = searchers.search(&q);
    assert_eq!(candidates[0].answer, json!(false));
}

struct FixtureServer {
    url: String,
    worker: std::thread::JoinHandle<Vec<String>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl FixtureServer {
    fn new(replies: Vec<(u16, String, &'static str)>) -> Self {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::{Duration, Instant};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body, extra_headers) in replies {
                let deadline = Instant::now() + Duration::from_secs(60);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                Instant::now() < deadline,
                                "expected local HTTP request never arrived"
                            );
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("local fixture accept failed: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                let mut received = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let amount = stream.read(&mut chunk).unwrap();
                    assert_ne!(amount, 0, "incomplete local HTTP request");
                    received.extend_from_slice(&chunk[..amount]);
                    if let Some(end) = received.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&received[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if received.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(received).unwrap());
                write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}", body.len()).unwrap();
            }
            while !worker_stop.load(std::sync::atomic::Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(30)))
                            .unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(30)))
                            .unwrap();
                        let mut extra = [0u8; 8192];
                        let amount = stream.read(&mut extra).unwrap_or(0);
                        requests.push(String::from_utf8_lossy(&extra[..amount]).into_owned());
                        let _ = write!(
                            stream,
                            "HTTP/1.1 500 Unexpected request\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("local fixture accept failed: {error}"),
                }
            }
            requests
        });
        Self { url, worker, stop }
    }

    fn finish(self) -> Vec<String> {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.worker.join().unwrap()
    }
}

#[test]
fn real_session_maps_requests_to_localhost_and_encodes_boolean_forms() {
    use cpass::transport::Session;
    let server = FixtureServer::new(vec![
        (200, "{\"status\":true}".into(), ""),
        (200, "{\"status\":true}".into(), ""),
    ]);
    let session = Session::new(30, 0, Some(&server.url)).unwrap();
    let response = session
        .get(
            "https://mooc1-api.chaoxing.com/fixture/read",
            &[("q".into(), "甲 & 乙".into())],
        )
        .unwrap();
    assert_eq!(response.json().unwrap()["status"], true);
    let q = question(QuestionType::TRUE_FALSE, Value::Null, json!(false));
    session
        .post_form(
            "https://mooc1-api.chaoxing.com/fixture/write",
            &[],
            &work_form(&[q]),
        )
        .unwrap();
    let requests = server.finish();
    assert!(requests[0].starts_with("GET /fixture/read?"));
    assert!(requests[1].starts_with("POST /fixture/write "));
    assert!(requests[1].contains("answer42=false"));
}

#[test]
fn http_searchers_preserve_raw_ai_output_and_continue_after_source_failure() {
    use cpass::search::Searchers;
    let raw = "答案：B\n解析：不正确的是乙";
    let server = FixtureServer::new(vec![
        (500, "{}".into(), ""),
        (200, json!({"data":false}).to_string(), ""),
        (
            200,
            json!({"choices":[{"message":{"content":raw}}]}).to_string(),
            "",
        ),
        (200, json!({"response":raw}).to_string(), ""),
    ]);
    let searchers = Searchers::new(&[
        json!({"type":"RestApiSearcher", "url":format!("{}/failed",server.url)}),
        json!({"type":"RestApiSearcher", "url":format!("{}/rest",server.url),"a_field":"$.data"}),
        json!({"type":"OpenAISearcher", "base_url":format!("{}/v1",server.url),"api_key":"fixture-only","model":"fixture","prompt":"{type}:{value} {options}","system_prompt":"回答题目"}),
        json!({"type":"OllamaSearcherAPI", "base_url":server.url,"model":"fixture","prompt":"{type}:{value} {options}","system_prompt":"回答题目"}),
    ]).unwrap();
    let candidates = searchers.search(&single());
    assert_eq!(candidates.len(), 4);
    assert!(candidates[0].error.is_some());
    assert_eq!(candidates[1].answer, json!(false));
    assert_eq!(candidates[2].answer, json!(raw));
    assert_eq!(candidates[3].answer, json!(raw));
    let requests = server.finish();
    assert!(requests[2].starts_with("POST /v1/chat/completions "));
    assert!(requests[3].starts_with("POST /api/generate "));
    for request in &requests[2..] {
        let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert!(body.to_string().contains("以下不正确的是？"));
        assert!(!request.to_ascii_lowercase().contains("\r\ncookie:"));
    }
}

fn fixture_searchers() -> cpass::search::Searchers {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/answers.json");
    cpass::search::Searchers::new(&[json!({"type":"JsonFileSearcher", "file_path":path})]).unwrap()
}

fn fixture_work(server: &FixtureServer) -> cpass::work::Work {
    use cpass::course::{Course, Task, TaskKind};
    let property = json!({"workid":"900", "_jobid":"job-900"});
    let task = Task {
        kind: TaskKind::Work,
        card_index: 0,
        chapter_id: 9,
        course: Course {
            id: 1,
            class_id: 2,
            cpi: 3,
            key: 4,
            name: "离线课程".into(),
            teacher: "教师".into(),
        },
        attachment: json!({"defaults":{"ktoken":"fixture-ktoken"},"attachments":[{"jobid":"job-900","enc":"fixture-enc","property":property.clone()}]}),
        property,
    };
    let session = cpass::transport::Session::new(30, 0, Some(&server.url)).unwrap();
    cpass::work::Work::from_task(session, &task, 7).unwrap()
}

fn judgment_page(work: bool, answer: &str) -> String {
    if work {
        format!(
            r#"<html><head><title>作业</title></head><body><h3 class="py-Title">离线作业</h3><form id="form1"><input id="workAnswerId" value="101"><input id="totalQuestionNum" value="1"><input id="workRelationId" value="102"><input id="fullScore" value="5"><input id="enc_work" value="fixture-enc"><div class="Py-mian1"><input id="answertype42" value="3"><div class="Py-m1-title">1.（判断题，5.0分）判断 题</div><input class="answerInput" id="answer42" value="{answer}"></div></form></body></html>"#
        )
    } else {
        format!(
            r#"<html><body><input id="ExamWaterMark" value="离线考生"><form id="submitTest"><input id="enc" value="fixture-enc"><input id="encRemainTime" value="100"><input id="remainTime" value="100"><input id="encLastUpdateTime" value="100"><div class="answerMain questionWrap singleQuesId ans-cc-exam" data="42"><input name="questionId" value="42"><input name="type42" value="3"><div class="tit"><h3>判断题（5.0分）</h3>1.判断 题</div><input id="answer42" value="{answer}"></div></form></body></html>"#
        )
    }
}

fn fixture_exam(server: &FixtureServer) -> cpass::exam::Exam {
    let session = cpass::transport::Session::new(30, 0, Some(&server.url)).unwrap();
    cpass::exam::Exam::new(
        session,
        cpass::exam::ExamInfo {
            id: 10,
            course_id: 1,
            class_id: 2,
            cpi: 3,
            enc_task: "fixture-task".into(),
            name: "离线考试".into(),
            status: "未开始".into(),
            expire_time: String::new(),
        },
        7,
    )
}

fn exam_start_replies() -> Vec<(u16, String, &'static str)> {
    vec![
        (200, include_str!("fixtures/exam_cover.html").into(), ""),
        (
            302,
            String::new(),
            "Location: /exam-ans/exam/test/reVersionTestStartNew?enc=fixture-start\r\n",
        ),
    ]
}

#[test]
fn work_workflow_dry_run_caches_only_and_successful_commit_has_real_receipt() {
    for commit in [false, true] {
        let mut replies = vec![(200, judgment_page(true, ""), "")];
        if commit {
            replies.push((200, json!({"status":true}).to_string(), ""));
        }
        let server = FixtureServer::new(replies);
        let mut work = fixture_work(&server);
        let (paper, report) =
            cpass::workflow::run_work(&mut work, &fixture_searchers(), commit, commit, false)
                .unwrap();
        assert_eq!(paper.questions[0].answer, json!(false));
        assert_eq!(report.matched, 1);
        assert_eq!(report.cached, 1);
        assert_eq!(report.submitted, 0);
        assert_eq!(report.incomplete, 0);
        assert_eq!(report.final_submitted, commit);
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        let requests = server.finish();
        assert_eq!(requests.len(), if commit { 2 } else { 1 });
        if commit {
            assert!(requests[1].starts_with("POST /work/addStudentWorkNew?"));
            assert!(requests[1].contains("answer42=false"));
            assert!(!requests[1].contains("tempsave=1"));
        }
    }
}

#[test]
fn work_save_failure_never_counts_as_saved_or_final_submitted() {
    let server = FixtureServer::new(vec![
        (200, judgment_page(true, "false"), ""),
        (
            200,
            json!({"status":false,"msg":"rejected"}).to_string(),
            "",
        ),
    ]);
    let mut work = fixture_work(&server);
    let (_, report) = cpass::workflow::run_work(
        &mut work,
        &cpass::search::Searchers::new(&[]).unwrap(),
        true,
        false,
        true,
    )
    .unwrap();
    assert_eq!(report.existing, 1);
    assert!(!report.saved && !report.final_submitted && !report.complete());
    assert!(!report.failures.is_empty());
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].starts_with("POST /work/addStudentWorkNew?"));
    assert!(requests[1].contains("tempsave=1"));
    assert!(requests[1].contains("answer42=false"));
    assert!(!requests[1].contains("keyboardDisplayRequiresUserAction"));
}

#[test]
fn work_workflow_missing_questions_never_final_submit_and_rejected_receipt_is_not_success() {
    let missing = judgment_page(true, "false").replace(
        "id=\"totalQuestionNum\" value=\"1\"",
        "id=\"totalQuestionNum\" value=\"2\"",
    );
    let server = FixtureServer::new(vec![(200, missing, "")]);
    let mut work = fixture_work(&server);
    let (_, report) = cpass::workflow::run_work(
        &mut work,
        &cpass::search::Searchers::new(&[]).unwrap(),
        true,
        true,
        false,
    )
    .unwrap();
    assert_eq!(report.existing, 1);
    assert!(report.incomplete > 0);
    assert!(!report.final_submitted);
    assert_eq!(server.finish().len(), 1);

    let server = FixtureServer::new(vec![
        (200, judgment_page(true, "false"), ""),
        (200, json!({"status":false}).to_string(), ""),
    ]);
    let mut work = fixture_work(&server);
    let (_, report) = cpass::workflow::run_work(
        &mut work,
        &cpass::search::Searchers::new(&[]).unwrap(),
        true,
        true,
        false,
    )
    .unwrap();
    assert!(!report.final_submitted);
    assert!(!report.failures.is_empty());
    assert_eq!(server.finish().len(), 2);
}

#[test]
fn exam_workflow_saved_false_skips_search_and_single_submit_but_verifies_each_page() {
    let mut replies = exam_start_replies();
    replies.extend([
        (200, judgment_page(false, "false"), ""),
        (
            200,
            "<ul><li data=\"0\" class=\"complated\">1</li></ul>".into(),
            "",
        ),
        (200, judgment_page(false, "false"), ""),
        (200, json!({"status":"success"}).to_string(), ""),
    ]);
    let server = FixtureServer::new(replies);
    let mut exam = fixture_exam(&server);
    exam.metadata().unwrap();
    exam.start(None).unwrap();
    let (paper, report) = cpass::workflow::run_exam(
        &mut exam,
        &cpass::search::Searchers::new(&[]).unwrap(),
        true,
        true,
        0.0,
    )
    .unwrap();
    assert_eq!(paper.questions[0].answer, json!(false));
    assert_eq!(report.existing, 1);
    assert_eq!(report.matched, 0);
    assert_eq!(report.submitted, 0);
    assert_eq!(report.incomplete, 0);
    assert!(report.final_submitted, "{:?}", report.failures);
    let requests = server.finish();
    assert_eq!(requests.len(), 6);
    assert!(requests[2].starts_with("GET /exam-ans/exam/phone/preview?"));
    assert!(requests[4].starts_with("GET /exam-ans/exam/test/reVersionTestStartNew?"));
    let posts = requests
        .iter()
        .filter(|r| r.starts_with("POST "))
        .collect::<Vec<_>>();
    assert_eq!(posts.len(), 1);
    assert!(posts[0].contains("tempSave=false"));
    assert!(!posts[0].contains("answer42="));
}

#[test]
fn exam_workflow_rejected_single_receipt_is_incomplete_and_never_final_submits() {
    let mut replies = exam_start_replies();
    replies.extend([
        (200, judgment_page(false, ""), ""),
        (200, "<ul><li data=\"0\">1</li></ul>".into(), ""),
        (200, judgment_page(false, ""), ""),
        (
            200,
            json!({"status":"error","msg":"fixture rejected"}).to_string(),
            "",
        ),
    ]);
    let server = FixtureServer::new(replies);
    let mut exam = fixture_exam(&server);
    exam.metadata().unwrap();
    exam.start(None).unwrap();
    let (_, report) =
        cpass::workflow::run_exam(&mut exam, &fixture_searchers(), true, true, 0.0).unwrap();
    assert_eq!(report.matched, 1);
    assert_eq!(report.submitted, 0);
    assert_eq!(report.incomplete, 1);
    assert!(!report.final_submitted);
    let requests = server.finish();
    assert_eq!(requests.len(), 6);
    assert!(requests[5].contains("tempSave=true"));
    assert!(requests[5].contains("answer42=false"));
    assert!(!requests.iter().any(|r| r.contains("tempSave=false")));
}

#[test]
fn exam_workflow_success_updates_session_parameters_before_final_submit() {
    let mut replies = exam_start_replies();
    replies.extend([
        (200, judgment_page(false, ""), ""),
        (200, "<ul><li data=\"0\">1</li></ul>".into(), ""),
        (200, judgment_page(false, ""), ""),
        (
            200,
            json!({"status":"success","data":"222|88|receipt-enc"}).to_string(),
            "",
        ),
        (200, json!({"status":"success"}).to_string(), ""),
    ]);
    let server = FixtureServer::new(replies);
    let mut exam = fixture_exam(&server);
    exam.metadata().unwrap();
    exam.start(None).unwrap();
    let (_, report) =
        cpass::workflow::run_exam(&mut exam, &fixture_searchers(), true, true, 0.0).unwrap();
    assert_eq!(report.submitted, 1);
    assert_eq!(report.incomplete, 0);
    assert!(report.final_submitted, "{:?}", report.failures);
    let requests = server.finish();
    assert_eq!(requests.len(), 7);
    assert!(requests[6].contains("enc=receipt-enc"));
    assert!(requests[6].contains("encLastUpdateTime=222"));
    assert!(requests[6].contains("encRemainTime=88"));
    assert!(requests[6].contains("tempSave=false"));
}

#[test]
fn exam_workflow_dry_run_only_previews_and_never_submits() {
    let mut replies = exam_start_replies();
    replies.push((200, judgment_page(false, ""), ""));
    replies.push((200, "<ul><li data=\"0\">1</li></ul>".into(), ""));
    let server = FixtureServer::new(replies);
    let mut exam = fixture_exam(&server);
    exam.metadata().unwrap();
    exam.start(None).unwrap();
    let (_, report) =
        cpass::workflow::run_exam(&mut exam, &fixture_searchers(), false, false, 0.0).unwrap();
    assert_eq!(report.matched, 1);
    assert_eq!(report.submitted, 0);
    assert!(!report.final_submitted);
    let requests = server.finish();
    assert_eq!(requests.len(), 4);
    assert!(requests.iter().all(|r| r.starts_with("GET ")));
}

struct CliDirectory(std::path::PathBuf);

impl CliDirectory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "cpass-cli-parity-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn command(&self) -> std::process::Command {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_cpass"));
        command.current_dir(&self.0);
        command
    }
}

impl Drop for CliDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_work_export_reviewed_is_read_only_and_keeps_reference_out_of_answers() {
    let folder = CliDirectory::new();
    let html = judgment_page(true, "false")
        .replace("<title>作业</title>", "<title>作业已批阅</title>")
        .replace("</div></form>", "<p>我的答案：false</p><p>正确答案：true</p><p>得分：0</p><p>解析：<span>正确答案：不可提取</span></p></div></form>");
    let account = json!({"result":1,"msg":{"puid":7,"name":"fixture","phone":"","schoolname":"school","sex":-1}});
    let server = FixtureServer::new(vec![(200, account.to_string(), ""), (200, html, "")]);
    let task = cpass::course::Task {
        kind: cpass::course::TaskKind::Work,
        card_index: 0,
        chapter_id: 9,
        course: cpass::course::Course {
            id: 1,
            class_id: 2,
            cpi: 3,
            key: 4,
            name: "fixture".into(),
            teacher: String::new(),
        },
        property: json!({"workid":"900","_jobid":"job-900"}),
        attachment: json!({"defaults":{"ktoken":"fixture-ktoken"},"attachments":[{"jobid":"job-900","enc":"fixture-enc","property":{"workid":"900"}}]}),
    };
    let input = folder.0.join("work-task.json");
    std::fs::write(&input, serde_json::to_vec(&task).unwrap()).unwrap();
    let output = folder
        .command()
        .args(["--base-url", &server.url, "work-export", "--task-file"])
        .arg(&input)
        .arg("--reviewed")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let review: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(review["status"], "reviewed");
    assert_eq!(review["questions"][0]["question_id"], 42);
    assert_eq!(review["questions"][0]["submitted_answer"], "false");
    assert_eq!(review["questions"][0]["reference_answer"], "true");
    assert_eq!(review["questions"][0]["score"], "0");
    assert!(review["questions"][0]["question"]["answer"].is_null());
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.starts_with("GET ")));
    assert!(requests[1].starts_with("GET /android/mworkspecial?"));
    assert!(
        !requests
            .iter()
            .any(|request| request.contains("addStudentWorkNew"))
    );
}

#[test]
fn cli_help_does_not_read_configuration_or_create_runtime_directories() {
    let folder = CliDirectory::new();
    let output = folder
        .command()
        .args(["--config", "missing.yml", "--help"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
    assert!(output.stderr.is_empty());
    assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 0);
}

#[test]
fn cli_parse_exports_legacy_work_and_exam_types_with_separate_reports() {
    let folder = CliDirectory::new();
    for (kind, file, export_type) in [
        ("work", "work_questions.html", 1),
        ("exam", "exam_questions.html", 0),
    ] {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(file);
        let output = folder
            .command()
            .args(["parse", "--kind", kind, "--input"])
            .arg(fixture)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let exported: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(exported["type"], export_type);
        assert_eq!(exported["questions"].as_array().unwrap().len(), 4);
        assert_eq!(exported["questions"][3]["answer"], false);
        let report: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(report["parsed"], 4);
        assert_eq!(report["parse_errors"], json!({}));
    }
    assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 0);
}

#[test]
fn cli_resolve_emits_one_json_document_on_stdout_and_report_on_stderr() {
    let folder = CliDirectory::new();
    let input = folder.0.join("input.json");
    std::fs::write(&input, json!({"id":"fixture","title":"离线判断","type":1,"questions":[{"id":42,"value":"判断 题","type":3,"options":null,"answer":null}]}).to_string()).unwrap();
    let answers =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/answers.json");
    let output = folder
        .command()
        .args(["resolve", "--input"])
        .arg(&input)
        .arg("--answers")
        .arg(answers)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exported: QuestionSet = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(exported.export_type, 1);
    assert_eq!(exported.questions[0].answer, false);
    let report: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(report["matched"], 1);
    assert_eq!(report["incomplete"], 0);
    assert_eq!(report["submitted"], 0);
    assert_eq!(report["final_submitted"], false);
    assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 1);
}

#[test]
fn cli_run_incomplete_tasks_fail_and_zero_task_counts_never_prove_completion() {
    for scenario in ["unknown", "incomplete-work", "zero-tasks"] {
        let folder = CliDirectory::new();
        let account = json!({"result":1,"msg":{"puid":7,"name":"fixture","phone":"","schoolname":"school","sex":-1}});
        let courses = json!({"result":1,"channelList":[{"cpi":3,"key":4,"content":{"id":2,"course":{"data":[{"id":1,"name":"fixture","teacherfactor":"teacher"}]}}}]});
        let chapters = json!({"data":[{"course":{"data":[{"knowledge":{"data":[{"id":9,"label":"1","name":"fixture","status":"1"}]}}]}}]});
        let count = if scenario == "zero-tasks" { 0 } else { 1 };
        let states = json!({"9":{"totalcount":count,"finishcount":count,"unfinishcount":0}});
        let description = match scenario {
            "unknown" => "<iframe module='future-module' data='{}'></iframe>",
            "incomplete-work" => {
                "<iframe module='work' data='{\"workid\":\"900\",\"_jobid\":\"job-900\"}'></iframe>"
            }
            _ => "",
        };
        let cards = json!({"data":[{"card":{"data":[{"description":description}]}}]});
        let mut replies = vec![
            (200, account.to_string(), ""),
            (200, courses.to_string(), ""),
            (200, chapters.to_string(), ""),
            (200, states.to_string(), ""),
            (200, cards.to_string(), ""),
        ];
        if scenario == "incomplete-work" {
            let attachment = json!({"defaults":{"ktoken":"fixture-ktoken"},"attachments":[{"jobid":"job-900","job":true,"enc":"fixture-enc","property":{"workid":"900"}}]});
            replies.push((
                200,
                format!("<script>window.AttachmentSetting = {attachment};</script>"),
                "",
            ));
            replies.push((200, judgment_page(true, ""), ""));
        }
        replies.push((200, chapters.to_string(), ""));
        replies.push((200, states.to_string(), ""));
        let expected_requests = replies.len();
        let server = FixtureServer::new(replies);
        let output = folder
            .command()
            .args(["--base-url", &server.url, "run", "--course-id", "1"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            scenario == "zero-tasks",
            "{scenario}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["platform_completion_verified"], false, "{scenario}");
        match scenario {
            "unknown" => {
                assert_eq!(result["tasks"][0]["kind"], "Unknown");
                assert_eq!(result["tasks"][0]["result"]["status"], "未完成");
            }
            "incomplete-work" => {
                assert_eq!(result["tasks"][0]["result"]["report"]["incomplete"], 1);
                assert_eq!(
                    result["tasks"][0]["result"]["report"]["final_submitted"],
                    false
                );
            }
            _ => {
                assert_eq!(result["tasks"], json!([]));
                assert_eq!(result["chapters_after"][0]["total"], 0);
                assert_eq!(result["chapters_after"][0]["finished"], 0);
            }
        }
        let requests = server.finish();
        assert_eq!(requests.len(), expected_requests, "{scenario}");
        assert!(
            !requests
                .iter()
                .any(|r| r.contains("/work/addStudentWorkNew"))
        );
    }
}

#[test]
fn cli_run_unexecuted_tasks_cannot_override_existing_platform_counts() {
    for scenario in [
        "preview-video",
        "disabled-video",
        "completed-video",
        "preview-work",
        "export-only-work",
    ] {
        let folder = CliDirectory::new();
        let commit = matches!(scenario, "disabled-video" | "export-only-work");
        let complete = scenario == "completed-video";
        let is_work = scenario.ends_with("work");
        let config = folder.0.join("config.yml");
        std::fs::write(
            &config,
            json!({
                "video":{"enable":scenario != "disabled-video","wait":0},
                "work":{"enable":scenario != "export-only-work","export":scenario == "export-only-work","wait":0},
                "document":{"wait":0},
                "request_retries":0
            }).to_string(),
        ).unwrap();
        let account = json!({"result":1,"msg":{"puid":7,"name":"fixture"}});
        let courses = json!({"result":1,"channelList":[{"cpi":3,"key":4,"content":{"id":2,"course":{"data":[{"id":1,"name":"fixture","teacherfactor":"teacher"}]}}}]});
        let chapters = json!({"data":[{"course":{"data":[{"knowledge":{"data":[{"id":9,"label":"1","name":"fixture","status":"1"}]}}]}}]});
        let states = json!({"9":{"totalcount":1,"finishcount":1,"unfinishcount":0}});
        let description = if is_work {
            "<iframe module='work' data='{\"workid\":\"900\",\"_jobid\":\"job-900\"}'></iframe>"
        } else {
            "<iframe module='insertvideo' data='{\"objectid\":\"video-900\"}'></iframe>"
        };
        let attachment = if is_work {
            json!({"defaults":{"ktoken":"fixture-ktoken"},"attachments":[{"jobid":"job-900","job":true,"enc":"fixture-enc","property":{"workid":"900"}}]})
        } else {
            json!({"defaults":{},"attachments":[{"jobid":"job-900","job":true,"isPassed":complete,"property":{"objectid":"video-900"}}]})
        };
        let mut replies = vec![
            (200, account.to_string(), ""),
            (200, courses.to_string(), ""),
            (200, chapters.to_string(), ""),
            (200, states.to_string(), ""),
        ];
        if commit {
            replies.push((200, String::new(), ""));
        }
        replies.extend([
            (
                200,
                json!({"data":[{"card":{"data":[{"description":description}]}}]}).to_string(),
                "",
            ),
            (
                200,
                format!("<script>window.AttachmentSetting = {attachment};</script>"),
                "",
            ),
        ]);
        if is_work {
            replies.push((200, judgment_page(true, "false"), ""));
        }
        replies.extend([
            (200, chapters.to_string(), ""),
            (200, states.to_string(), ""),
        ]);
        let expected_requests = replies.len();
        let server = FixtureServer::new(replies);
        let mut command = folder.command();
        command.arg("--config").arg(&config).args([
            "--base-url",
            &server.url,
            "run",
            "--course-id",
            "1",
        ]);
        if commit {
            command.arg("--commit");
        }
        let output = command.output().unwrap();
        assert_eq!(
            output.status.success(),
            complete,
            "{scenario}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["failed"], !complete, "{scenario}");
        assert_eq!(
            result["platform_completion_verified"], complete,
            "{scenario}"
        );
        assert_eq!(
            result["tasks"][0]["result"]["status"],
            if complete {
                "已有完成状态"
            } else {
                "未完成"
            },
            "{scenario}"
        );
        assert_eq!(result["chapters_after"][0]["finished"], 1);
        if is_work {
            assert_eq!(result["tasks"][0]["result"]["report"]["existing"], 1);
            assert_eq!(result["tasks"][0]["result"]["report"]["incomplete"], 0);
            assert_eq!(result["tasks"][0]["result"]["report"]["saved"], false);
        }
        let requests = server.finish();
        assert_eq!(requests.len(), expected_requests, "{scenario}");
        assert!(
            !requests
                .iter()
                .any(|request| request.contains("/work/addStudentWorkNew")
                    || request.contains("/multimedia/log/")
                    || request.contains("/ananas/status/")),
            "{scenario}"
        );
    }
}

#[test]
fn cli_run_chapter_refresh_failure_does_not_stop_later_tasks() {
    let folder = CliDirectory::new();
    let config = folder.0.join("config.yml");
    std::fs::write(&config, "request_retries: 0\ndocument:\n  wait: 0\n").unwrap();
    let account = json!({"result":1,"msg":{"puid":7,"name":"fixture"}});
    let courses = json!({"result":1,"channelList":[{"cpi":3,"key":4,"content":{"id":2,"course":{"data":[{"id":1,"name":"fixture","teacherfactor":"teacher"}]}}}]});
    let chapters = json!({"data":[{"course":{"data":[{"knowledge":{"data":[{"id":9,"label":"1","name":"first","status":"1"},{"id":10,"label":"2","name":"second","status":"1"}]}}]}}]});
    let states = json!({"9":{"totalcount":1,"finishcount":1,"unfinishcount":0},"10":{"totalcount":1,"finishcount":1,"unfinishcount":0}});
    let cards = json!({"data":[{"card":{"data":[{"description":"<iframe module='insertdoc' data='{\"objectid\":\"doc-10\"}'></iframe>"}]}}]});
    let attachment = json!({"defaults":{},"attachments":[{"job":true,"jobid":"doc-job-10","jtoken":"fixture-token","property":{"objectid":"doc-10"}}]});
    let server = FixtureServer::new(vec![
        (200, account.to_string(), ""),
        (200, courses.to_string(), ""),
        (200, chapters.to_string(), ""),
        (200, states.to_string(), ""),
        (503, "refresh unavailable".into(), ""),
        (200, String::new(), ""),
        (200, cards.to_string(), ""),
        (
            200,
            format!("<script>window.AttachmentSetting = {attachment};</script>"),
            "",
        ),
        (200, json!({"status":true}).to_string(), ""),
        (200, chapters.to_string(), ""),
        (200, states.to_string(), ""),
    ]);
    let output = folder
        .command()
        .arg("--config")
        .arg(&config)
        .args([
            "--base-url",
            &server.url,
            "run",
            "--course-id",
            "1",
            "--commit",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["failed"], true);
    assert_eq!(result["platform_completion_verified"], false);
    assert_eq!(result["tasks"].as_array().unwrap().len(), 2);
    assert_eq!(result["tasks"][0]["chapter"], 9);
    assert_eq!(result["tasks"][0]["status"], "未完成");
    assert!(
        result["tasks"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("503")
    );
    assert_eq!(result["tasks"][1]["chapter"], 10);
    assert_eq!(result["tasks"][1]["kind"], "Document");
    assert_eq!(result["tasks"][1]["result"]["status"], true);
    let requests = server.finish();
    assert_eq!(requests.len(), 11);
    assert!(requests[4].starts_with("GET /mooc-ans/mycourse/studentstudyAjax?"));
    assert!(requests[4].contains("chapterId=9"));
    assert!(requests[5].starts_with("GET /mooc-ans/mycourse/studentstudyAjax?"));
    assert!(requests[5].contains("chapterId=10"));
    assert!(requests[6].starts_with("GET /gas/knowledge?id=10&"));
    assert!(requests[8].starts_with("GET /ananas/job/document?"));
    assert!(requests[8].contains("knowledgeid=10"));
}

#[test]
fn cli_batch_resume_rechecks_each_course_before_skipping_checkpoint_successes() {
    let folder = CliDirectory::new();
    let checkpoint = folder.0.join("checkpoint.json");
    std::fs::write(&checkpoint, json!({"version":1,"uid":7,"course_ids":[1,2],"class_id":null,"reports":[{"platform_completion_verified":true},{"platform_completion_verified":true}]}).to_string()).unwrap();
    let config = folder.0.join("config.yml");
    std::fs::write(&config, "log_path: logs\nrequest_retries: 0\n").unwrap();
    let account = json!({"result":1,"msg":{"puid":7,"name":"private-account-name","phone":"13900009999","schoolname":"school","sex":-1}});
    let courses = json!({"result":1,"channelList":[
        {"cpi":3,"key":11,"content":{"id":21,"course":{"data":[{"id":1,"name":"first","teacherfactor":"teacher"}]}}},
        {"cpi":4,"key":12,"content":{"id":22,"course":{"data":[{"id":2,"name":"second","teacherfactor":"teacher"}]}}}
    ]});
    let chapter = |id| json!({"data":[{"course":{"data":[{"knowledge":{"data":[{"id":id,"label":"1","name":"fixture","status":"1"}]}}]}}]});
    let first_done = json!({"91":{"totalcount":1,"finishcount":1,"unfinishcount":0}});
    let second_pending = json!({"92":{"totalcount":1,"finishcount":0,"unfinishcount":1}});
    let cards = json!({"data":[{"card":{"data":[{"description":"<iframe module='future-module' data='{}'></iframe>"}]}}]});
    let server = FixtureServer::new(vec![
        (200, account.to_string(), ""),
        (200, courses.to_string(), ""),
        (200, chapter(91).to_string(), ""),
        (200, first_done.to_string(), ""),
        (200, courses.to_string(), ""),
        (200, chapter(92).to_string(), ""),
        (200, second_pending.to_string(), ""),
        (200, chapter(92).to_string(), ""),
        (200, second_pending.to_string(), ""),
        (200, cards.to_string(), ""),
        (200, chapter(92).to_string(), ""),
        (200, second_pending.to_string(), ""),
    ]);
    let output = folder
        .command()
        .arg("--config")
        .arg(&config)
        .args([
            "--base-url",
            &server.url,
            "run-batch",
            "--course-id",
            "1",
            "2",
            "--resume",
        ])
        .arg(&checkpoint)
        .env("CPASS_PASSWORD", "fixture-password-secret")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["reports"].as_array().unwrap().len(), 2);
    assert_eq!(result["reports"][0]["course"]["id"], 1);
    assert_eq!(result["reports"][0]["skipped"], true);
    assert_eq!(result["reports"][0]["platform_completion_verified"], true);
    assert_eq!(result["reports"][1]["course"]["id"], 2);
    assert_ne!(result["reports"][1]["skipped"], true);
    assert_eq!(result["reports"][1]["tasks"][0]["kind"], "Unknown");
    assert_eq!(result["reports"][1]["platform_completion_verified"], false);
    assert_eq!(result["failed"], true);
    let saved: Value = serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();
    assert_eq!(saved["uid"], 7);
    assert_eq!(saved["course_ids"], json!([1, 2]));
    assert_eq!(saved["reports"], result["reports"]);
    let requests = server.finish();
    assert_eq!(requests.len(), 12);
    let cards = requests
        .iter()
        .filter(|r| r.starts_with("GET /gas/knowledge?"))
        .collect::<Vec<_>>();
    assert_eq!(cards.len(), 1);
    assert!(cards[0].lines().next().unwrap().contains("courseid=2"));
    assert!(
        !requests
            .iter()
            .any(|r| r.contains("/work/addStudentWorkNew"))
    );
    let events = std::fs::read_to_string(folder.0.join("logs/events.jsonl")).unwrap();
    assert!(!events.trim().is_empty());
    for line in events.lines() {
        serde_json::from_str::<cpass::operations::Event>(line).unwrap();
    }
    for secret in [
        "private-account-name",
        "13900009999",
        "fixture-password-secret",
    ] {
        assert!(!events.contains(secret));
        assert!(!String::from_utf8_lossy(&output.stdout).contains(secret));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
    }
    assert!(String::from_utf8_lossy(&output.stderr).contains("批量任务有未完成或失败"));
}

#[test]
fn cli_batch_rejects_checkpoint_for_wrong_account_or_courses_before_task_requests() {
    for (uid, course_ids) in [(8, json!([1, 2])), (7, json!([1, 9]))] {
        let folder = CliDirectory::new();
        let checkpoint = folder.0.join("checkpoint.json");
        let original =
            json!({"version":1,"uid":uid,"course_ids":course_ids,"class_id":null,"reports":[]})
                .to_string();
        std::fs::write(&checkpoint, &original).unwrap();
        let server = FixtureServer::new(vec![(
            200,
            json!({"result":1,"msg":{"puid":7,"name":"fixture"}}).to_string(),
            "",
        )]);
        let output = folder
            .command()
            .args([
                "--base-url",
                &server.url,
                "run-batch",
                "--course-id",
                "1",
                "2",
                "--resume",
            ])
            .arg(&checkpoint)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("checkpoint 与当前账户或课程不一致")
        );
        assert_eq!(std::fs::read_to_string(&checkpoint).unwrap(), original);
        let requests = server.finish();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("GET /apis/login/userLogin4Uname.do "));
        assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 1);
    }
}

#[test]
fn cli_tui_zero_and_eof_exit_without_network_or_runtime_files() {
    use std::io::Write;
    use std::process::Stdio;
    for input in [b"0\n".as_slice(), b"".as_slice()] {
        let folder = CliDirectory::new();
        let server = FixtureServer::new(vec![]);
        let mut child = folder
            .command()
            .args(["--base-url", &server.url, "tui"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("0 退出"));
        assert!(server.finish().is_empty());
        assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 0);
    }
}

#[test]
fn cli_tui_course_write_prompts_control_preview_save_and_final_submission() {
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::process::Stdio;

    for (input, commit, final_submit) in [
        ("5\n1\n\nno\n0\n", false, false),
        ("5\n1\n\nyes\nno\n0\n", true, false),
        ("5\n1\n\nyes\nyes\n0\n", true, true),
    ] {
        let folder = CliDirectory::new();
        std::fs::write(
            folder.0.join("config.yml"),
            "request_retries: 0\nwork:\n  wait: 0\n",
        )
        .unwrap();
        let account = json!({"result":1,"msg":{"puid":7,"name":"fixture","phone":"","schoolname":"school","sex":-1}});
        let courses = json!({"result":1,"channelList":[{"cpi":3,"key":4,"content":{"id":2,"course":{"data":[{"id":1,"name":"fixture","teacherfactor":"teacher"}]}}}]});
        let chapters = json!({"data":[{"course":{"data":[{"knowledge":{"data":[{"id":9,"label":"1","name":"fixture","status":"1"}]}}]}}]});
        let states = json!({"9":{"totalcount":1,"finishcount":0,"unfinishcount":1}});
        let cards = json!({"data":[{"card":{"data":[{"description":"<iframe module='work' data='{\"workid\":\"900\",\"_jobid\":\"job-900\"}'></iframe>"}]}}]});
        let attachment = json!({"defaults":{"ktoken":"fixture-ktoken"},"attachments":[{"jobid":"job-900","job":true,"enc":"fixture-enc","property":{"workid":"900"}}]});
        let mut replies = vec![
            (200, account.to_string(), ""),
            (200, courses.to_string(), ""),
            (200, chapters.to_string(), ""),
            (200, states.to_string(), ""),
        ];
        let mut expected_paths = vec![
            ("GET", "/apis/login/userLogin4Uname.do"),
            ("GET", "/mycourse/backclazzdata"),
            ("GET", "/gas/clazz"),
            ("POST", "/job/myjobsnodesmap"),
        ];
        if commit {
            replies.push((200, String::new(), ""));
            expected_paths.push(("GET", "/mooc-ans/mycourse/studentstudyAjax"));
        }
        replies.extend([
            (200, cards.to_string(), ""),
            (
                200,
                format!("<script>window.AttachmentSetting = {attachment};</script>"),
                "",
            ),
            (200, judgment_page(true, "false"), ""),
        ]);
        expected_paths.extend([
            ("GET", "/gas/knowledge"),
            ("GET", "/knowledge/cards"),
            ("GET", "/android/mworkspecial"),
        ]);
        if commit {
            replies.push((200, json!({"status":true}).to_string(), ""));
            expected_paths.push(("POST", "/work/addStudentWorkNew"));
        }
        replies.extend([
            (200, chapters.to_string(), ""),
            (200, states.to_string(), ""),
        ]);
        expected_paths.extend([("GET", "/gas/clazz"), ("POST", "/job/myjobsnodesmap")]);
        let server = FixtureServer::new(replies);
        let mut child = folder
            .command()
            .args(["--base-url", &server.url, "tui"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{input:?}: {stderr}");
        assert!(stderr.contains("课程ID: ") && stderr.contains("班级ID（可留空）: "));
        assert!(stderr.contains("写入平台请输入 yes，其他输入仅查看: "));
        assert_eq!(
            stderr.contains("最终交作业请输入 yes，其他输入仅保存: "),
            commit
        );
        assert_eq!(stderr.contains("操作完成"), commit, "{input:?}: {stderr}");
        assert_eq!(
            stderr.contains("有任务未完成或提交失败"),
            !commit,
            "{input:?}: {stderr}"
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        let report = &result["tasks"][0]["result"]["report"];
        assert_eq!(report["existing"], 1);
        assert_eq!(report["saved"], commit && !final_submit);
        assert_eq!(report["final_submitted"], final_submit);
        assert_eq!(report["incomplete"], 0);
        assert_eq!(result["platform_completion_verified"], false);
        let requests = server.finish();
        assert_eq!(requests.len(), expected_paths.len(), "{input:?}");
        let mut writes = 0;
        for (request, (method, path)) in requests.iter().zip(expected_paths) {
            let mut line = request.lines().next().unwrap().split_whitespace();
            assert_eq!(line.next().unwrap(), method);
            let target =
                reqwest::Url::parse(&format!("http://fixture{}", line.next().unwrap())).unwrap();
            assert_eq!(target.path(), path);
            if path != "/work/addStudentWorkNew" {
                continue;
            }
            writes += 1;
            let query: BTreeMap<_, _> = target.query_pairs().into_owned().collect();
            let body = request.split_once("\r\n\r\n").unwrap().1;
            let form: BTreeMap<_, _> = reqwest::Url::parse(&format!("http://fixture/?{body}"))
                .unwrap()
                .query_pairs()
                .into_owned()
                .collect();
            assert_eq!(query["workAnswerId"], "101");
            assert_eq!(form["workAnswerId"], "101");
            assert_eq!(form["answer42"], "false");
            assert_eq!(form["answertype42"], "3");
            assert_eq!(form["totalQuestionNum"], "1");
            if final_submit {
                assert!(!query.contains_key("tempsave"));
                assert!(!query.contains_key("saveStatus"));
                assert_eq!(query["workid"], "102");
                assert_eq!(query["jobid"], "job-900");
                assert_eq!(form["pyFlag"], "");
            } else {
                assert_eq!(query["tempsave"], "1");
                assert_eq!(query["saveStatus"], "1");
                assert_eq!(form["pyFlag"], "1");
            }
        }
        assert_eq!(writes, usize::from(commit));
    }
}

#[test]
fn cli_resources_and_subjective_review_remain_offline_and_never_fill_an_answer() {
    let folder = CliDirectory::new();
    let html = r#"<html><img src="/logo.png"><div class="Py-mian1" data="42"><input id="answertype42" value="4"><div class="Py-m1-title">解释<img src="/formula.png" alt="公式"><math><mi>x</mi><mo>+</mo><mn>1</mn></math><script type="math/tex">x+1</script></div></div></html>"#;
    let input_html = folder.0.join("question.html");
    std::fs::write(&input_html, html).unwrap();
    let server = FixtureServer::new(vec![]);
    let output = folder
        .command()
        .args(["resources", "--input"])
        .arg(&input_html)
        .args(["--base-url", &server.url])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let manifest: Value = serde_json::from_slice(&output.stdout).unwrap();
    let resources = manifest["resources"].as_array().unwrap();
    assert_eq!(resources.len(), 3);
    assert_eq!(resources[0]["kind"], "image");
    assert_eq!(
        resources[0]["source"],
        format!("{}/formula.png", server.url)
    );
    assert_eq!(resources[0]["alt"], "公式");
    assert_eq!(resources[1]["kind"], "mathml");
    assert!(
        resources[1]["content"]
            .as_str()
            .unwrap()
            .contains("<mi>x</mi>")
    );
    assert_eq!(resources[2]["kind"], "tex");
    assert_eq!(resources[2]["content"], "x+1");
    for resource in resources {
        assert_eq!(resource["question_id"], 42);
        assert!(resource["local_file"].is_null());
        assert!(resource.get("answer").is_none());
    }
    assert_eq!(std::fs::read_to_string(&input_html).unwrap(), html);
    assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 1);

    let input = folder.0.join("subjective.json");
    let source = json!({"id":"review","title":"仅人审","type":1,"questions":[{"id":42,"value":"解释 x+1","type":4,"options":null,"answer":null},{"id":43,"value":"单选","type":0,"options":{"A":"甲"},"answer":"A"}]}).to_string();
    std::fs::write(&input, &source).unwrap();
    std::fs::write(
        folder.0.join("answers.json"),
        json!({"解释 x+1":"待审核的解释草稿"}).to_string(),
    )
    .unwrap();
    let config = folder.0.join("config.yml");
    std::fs::write(
        &config,
        "log_path: logs\nsearchers:\n  - type: JsonFileSearcher\n    file_path: answers.json\n",
    )
    .unwrap();
    let output = folder
        .command()
        .arg("--config")
        .arg(&config)
        .args(["--base-url", &server.url, "review", "--suggest", "--input"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let review: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(review["requires_human_review"], true);
    assert_eq!(review["questions"].as_array().unwrap().len(), 1);
    let question = &review["questions"][0];
    assert_eq!(question["requires_human_review"], true);
    assert_eq!(question["automatic_submission"], false);
    assert!(question["question"]["answer"].is_null());
    assert_eq!(question["drafts"][0]["answer"], "待审核的解释草稿");
    assert_eq!(std::fs::read_to_string(&input).unwrap(), source);
    assert!(server.finish().is_empty());
    let events = std::fs::read_to_string(folder.0.join("logs/events.jsonl")).unwrap();
    assert!(!events.contains("待审核的解释草稿"));
    for line in events.lines() {
        serde_json::from_str::<cpass::operations::Event>(line).unwrap();
    }
}

#[test]
fn cli_notification_receipts_are_diagnosable_without_changing_business_results() {
    for accepted in [true, false] {
        let folder = CliDirectory::new();
        let business = FixtureServer::new(vec![(
            200,
            json!({"result":1,"msg":{"puid":7,"name":"private-name","phone":"13800138000","schoolname":"school","sex":-1}}).to_string(),
            "",
        )]);
        let notifier = FixtureServer::new(vec![(
            if accepted { 200 } else { 500 },
            if accepted {
                json!({"id":42})
            } else {
                json!({"message":"PRIVATE_NOTIFICATION_RESPONSE"})
            }
            .to_string(),
            "",
        )]);
        let config = folder.0.join("config.yml");
        std::fs::write(&config, json!({
            "log_path":"logs",
            "notifications":{"enabled":true,"gotify":{"url":notifier.url,"token_env":"CPASS_TEST_NOTIFIER_TOKEN"}}
        }).to_string()).unwrap();
        let output = folder
            .command()
            .arg("--config")
            .arg(&config)
            .args(["--base-url", &business.url, "account"])
            .env("CPASS_TEST_NOTIFIER_TOKEN", "PRIVATE_NOTIFICATION_TOKEN")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let account: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(account["puid"], 7);
        assert_eq!(output.stderr.is_empty(), accepted);
        let diagnosis = folder.0.join("diagnostics.json");
        let diagnosed = folder
            .command()
            .arg("--config")
            .arg(&config)
            .args(["diagnose", "--output"])
            .arg(&diagnosis)
            .env("CPASS_TEST_NOTIFIER_TOKEN", "PRIVATE_NOTIFICATION_TOKEN")
            .output()
            .unwrap();
        assert!(
            diagnosed.status.success(),
            "{}",
            String::from_utf8_lossy(&diagnosed.stderr)
        );
        let raw = std::fs::read_to_string(&diagnosis).unwrap();
        let bundle: Value = serde_json::from_str(&raw).unwrap();
        let events = bundle["events"].as_array().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["stage"], "command");
        assert_eq!(events[0]["outcome"], "succeeded");
        assert!(events[0].get("notification").is_none());
        assert_eq!(events[1]["stage"], "notify");
        assert_eq!(
            events[1]["outcome"],
            if accepted { "succeeded" } else { "failed" }
        );
        assert_eq!(
            events[1]["notification"],
            json!({"provider":"gotify","accepted":accepted,"error":if accepted { Value::Null } else { json!("receipt") }})
        );
        assert_eq!(bundle["discarded_lines"], 0);
        for private in [
            "PRIVATE_NOTIFICATION_TOKEN",
            "PRIVATE_NOTIFICATION_RESPONSE",
            "private-name",
            "13800138000",
        ] {
            assert!(!raw.contains(private));
            assert!(!String::from_utf8_lossy(&output.stderr).contains(private));
        }
        let requests = notifier.finish();
        assert_eq!(
            requests.len(),
            1,
            "receipt logging or diagnose sent another notification"
        );
        assert!(requests[0].starts_with("POST /message HTTP/1.1"));
        assert_eq!(business.finish().len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let log = folder.0.join("logs/events.jsonl");
            assert_eq!(
                std::fs::metadata(log).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

#[test]
fn cli_offline_commands_keep_local_logs_without_connecting_to_enabled_mqtt() {
    use std::net::TcpListener;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let folder = CliDirectory::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let broker = format!("mqtt://{}", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker = std::thread::spawn(move || {
        let mut connections = 0;
        while !worker_stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    connections += 1;
                    drop(stream);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5))
                }
                Err(error) => panic!("MQTT test listener failed: {error}"),
            }
        }
        connections
    });
    let config = folder.0.join("config.yml");
    std::fs::write(&config, json!({"log_path":"logs","notifications":{"enabled":true,"mqtt":{"broker":broker,"topic":"cpass/test","allow_plaintext":true}}}).to_string()).unwrap();
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let parsed = folder
        .command()
        .arg("--config")
        .arg(&config)
        .args(["parse", "--kind", "work", "--input"])
        .arg(fixtures.join("work_questions.html"))
        .output()
        .unwrap();
    let input = folder.0.join("input.json");
    std::fs::write(&input, json!({"id":"fixture","title":"离线判断","type":1,"questions":[{"id":42,"value":"判断 题","type":3,"options":null,"answer":null}]}).to_string()).unwrap();
    let resolved = folder
        .command()
        .arg("--config")
        .arg(&config)
        .args(["resolve", "--input"])
        .arg(&input)
        .arg("--answers")
        .arg(fixtures.join("answers.json"))
        .output()
        .unwrap();
    stop.store(true, Ordering::Relaxed);
    assert_eq!(
        worker.join().unwrap(),
        0,
        "offline CLI attempted MQTT notification"
    );
    for output in [&parsed, &resolved] {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<QuestionSet>(&output.stdout).unwrap();
        serde_json::from_slice::<Value>(&output.stderr).unwrap();
    }
    let report: Value = serde_json::from_slice(&parsed.stderr).unwrap();
    assert_eq!(report["parsed"], 4);
    let report: Value = serde_json::from_slice(&resolved.stderr).unwrap();
    assert_eq!(report["matched"], 1);
    assert_eq!(report["incomplete"], 0);
    let result: QuestionSet = serde_json::from_slice(&resolved.stdout).unwrap();
    assert_eq!(result.questions[0].answer, false);
    let logs = std::fs::read_to_string(folder.0.join("logs/events.jsonl")).unwrap();
    assert!(logs.lines().count() >= 2);
    for line in logs.lines() {
        let event = serde_json::from_str::<cpass::operations::Event>(line).unwrap();
        assert!(event.notification.is_none());
    }
}

#[test]
fn cli_batch_output_aliases_cannot_overwrite_resume_checkpoint() {
    #[cfg(not(unix))]
    let aliases = ["./checkpoint.json"];
    #[cfg(unix)]
    let aliases = ["./checkpoint.json", "checkpoint-link.json"];
    for alias in aliases {
        let folder = CliDirectory::new();
        let checkpoint = folder.0.join("checkpoint.json");
        let original = json!({"version":1,"uid":7,"course_ids":[1,2],"class_id":null,"reports":[{"preserved":true}]}).to_string();
        std::fs::write(&checkpoint, &original).unwrap();
        #[cfg(unix)]
        if alias == "checkpoint-link.json" {
            std::os::unix::fs::symlink("checkpoint.json", folder.0.join(alias)).unwrap();
        }
        let server = FixtureServer::new(vec![(
            200,
            json!({"result":1,"msg":{"puid":7,"name":"fixture"}}).to_string(),
            "",
        )]);
        let output = folder
            .command()
            .args([
                "--base-url",
                &server.url,
                "run-batch",
                "--course-id",
                "1",
                "2",
                "--resume",
                "checkpoint.json",
                "--output",
                alias,
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("checkpoint") && error.contains("output"),
            "{error}"
        );
        assert_eq!(std::fs::read_to_string(&checkpoint).unwrap(), original);
        assert_eq!(
            std::fs::read_to_string(folder.0.join(alias)).unwrap(),
            original
        );
        let requests = server.finish();
        assert_eq!(
            requests.len(),
            1,
            "no course or task requests are permitted"
        );
        assert!(requests[0].starts_with("GET /apis/login/userLogin4Uname.do "));
        #[cfg(unix)]
        if alias == "checkpoint-link.json" {
            assert!(
                std::fs::symlink_metadata(folder.0.join(alias))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
    }
    #[cfg(windows)]
    {
        let folder = CliDirectory::new();
        let checkpoint = folder.0.join("Checkpoint-new.json");
        let report = folder.0.join("checkpoint-new.json");
        let server = FixtureServer::new(vec![(
            200,
            json!({"result":1,"msg":{"puid":7,"name":"fixture"}}).to_string(),
            "",
        )]);
        let output = folder
            .command()
            .args([
                "--base-url",
                &server.url,
                "run-batch",
                "--course-id",
                "1",
                "2",
                "--resume",
                "Checkpoint-new.json",
                "--output",
                "checkpoint-new.json",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("checkpoint") && error.contains("output"),
            "{error}"
        );
        assert!(!checkpoint.exists());
        assert!(!report.exists());
        let requests = server.finish();
        assert_eq!(
            requests.len(),
            1,
            "no course or task requests are permitted"
        );
        assert!(requests[0].starts_with("GET /apis/login/userLogin4Uname.do "));
    }
}
