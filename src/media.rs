use crate::{
    course::{Task, TaskKind, scalar},
    transport::{Session, timestamp},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    thread,
    time::{Duration, Instant},
};

pub fn video_signature(
    class_id: u64,
    uid: u64,
    job: &str,
    object: &str,
    playing: u64,
    duration: u64,
) -> String {
    format!(
        "{:x}",
        md5::compute(format!(
            "[{class_id}][{uid}][{job}][{object}][{}][d_yHJ!$pdA~5][{}][0_{duration}]",
            u128::from(playing) * 1000,
            u128::from(duration) * 1000
        ))
    )
}

fn required(value: &Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(scalar)
        .with_context(|| format!("missing media field {key}"))
}

fn successful_receipt(value: &Value) -> bool {
    let error = value
        .get("error")
        .is_some_and(|v| !v.is_null() && v != false && v != "" && v != 0);
    !error
        && ["status", "success", "result", "isPassed"]
            .iter()
            .any(|key| {
                value
                    .get(key)
                    .is_some_and(|v| v == true || v == 1 || v == "success")
            })
}

// This endpoint signs a legacy query where otherInfo embeds unescaped '&' and '='.
fn media_query(params: &[(String, String)]) -> String {
    fn encode(text: &str) -> String {
        let mut out = String::new();
        for byte in text.bytes() {
            match byte {
                b'A'..=b'Z'
                | b'a'..=b'z'
                | b'0'..=b'9'
                | b'_'
                | b'.'
                | b'-'
                | b'~'
                | b'&'
                | b'=' => out.push(byte as char),
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{byte:02X}")),
            }
        }
        out
    }
    params
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn run_video(
    session: &Session,
    task: &Task,
    uid: u64,
    speed: f64,
    report_rate: u64,
) -> Result<Value> {
    ensure!(task.kind == TaskKind::Video, "task is not video");
    ensure!(
        speed.is_finite() && speed > 0.0,
        "video speed must be positive and finite"
    );
    ensure!(
        (1..=3600).contains(&report_rate),
        "video report interval must be 1..3600 seconds"
    );
    if task.already_complete() {
        return Ok(json!({"status":true,"already_complete":true}));
    }
    let point = task
        .point()
        .context("video attachment missing or ambiguous")?;
    let object = required(&task.property, "objectid")?;
    ensure!(
        object
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
        "invalid video object identifier"
    );
    let job = required(point, "jobid")?;
    let other = required(point, "otherInfo")?;
    let fid = required(&task.attachment["defaults"], "fid")?;
    let status = session
        .get(
            &format!("https://mooc1-api.chaoxing.com/ananas/status/{object}"),
            &[
                ("k".into(), fid),
                ("flag".into(), "normal".into()),
                ("_dc".into(), timestamp().to_string()),
            ],
        )?
        .json()?;
    ensure!(status["status"] == "success", "video status rejected");
    let token = required(&status, "dtoken")?;
    ensure!(
        token
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
        "invalid video resource token"
    );
    let duration: u64 = required(&status, "duration")?
        .parse()
        .context("invalid video duration")?;
    let wall_seconds = duration as f64 / speed;
    // ponytail: one-day run/10,000-report bounds; longer media needs an explicit resumable runner.
    ensure!(
        duration > 0 && duration <= 604800 && wall_seconds.is_finite() && wall_seconds <= 86400.0,
        "video duration exceeds bounded runner limits"
    );
    let reports = (wall_seconds / report_rate as f64).ceil() as u64 + 1;
    ensure!(reports <= 10000, "video would exceed 10000 reports");
    let rt = point["property"]
        .get("rt")
        .and_then(scalar)
        .unwrap_or_else(|| "0.9".into());
    let rt_number: f64 = rt.parse().context("invalid video rt")?;
    ensure!(rt_number.is_finite() && rt_number > 0.0, "invalid video rt");
    let start = Instant::now();
    let mut previous = 0;
    for attempt in 0..reports {
        let playing = if attempt == 0 {
            0
        } else {
            ((start.elapsed().as_secs_f64() * speed).floor() as u64)
                .min(duration)
                .max(previous)
        };
        previous = playing;
        let params = vec![
            ("otherInfo".into(), other.clone()),
            ("playingTime".into(), playing.to_string()),
            ("duration".into(), duration.to_string()),
            ("jobid".into(), job.clone()),
            ("clipTime".into(), format!("0_{duration}")),
            ("clazzId".into(), task.course.class_id.to_string()),
            ("objectId".into(), object.clone()),
            ("userid".into(), uid.to_string()),
            ("isdrag".into(), "0".into()),
            (
                "enc".into(),
                video_signature(task.course.class_id, uid, &job, &object, playing, duration),
            ),
            ("rt".into(), rt.clone()),
            ("dtype".into(), "Video".into()),
            ("view".into(), "pc".into()),
            ("_t".into(), timestamp().to_string()),
        ];
        let url = format!(
            "https://mooc1-api.chaoxing.com/multimedia/log/a/{}/{token}?{}",
            task.course.cpi,
            media_query(&params)
        );
        let receipt = session.get(&url, &[])?.json()?;
        ensure!(
            receipt
                .get("error")
                .is_none_or(|v| v.is_null() || v == false || v == "" || v == 0),
            "video report rejected"
        );
        match receipt["isPassed"].as_bool() {
            Some(true) => return Ok(receipt),
            Some(false) => (),
            None => bail!("video report missing isPassed receipt"),
        }
        ensure!(
            playing < duration,
            "video reached duration without server completion receipt"
        );
        let remaining = (wall_seconds - start.elapsed().as_secs_f64()).max(0.0);
        thread::sleep(Duration::from_secs_f64(remaining.min(report_rate as f64)));
    }
    bail!("video report limit reached without completion receipt")
}

pub fn run_document(session: &Session, task: &Task) -> Result<Value> {
    ensure!(task.kind == TaskKind::Document, "task is not document");
    if task.already_complete() {
        return Ok(json!({"status":true,"already_complete":true}));
    }
    let point = task
        .point()
        .context("document attachment missing or ambiguous")?;
    ensure!(point["job"] == true, "document has no verified active job");
    let receipt = session
        .get(
            "https://mooc1.chaoxing.com/ananas/job/document",
            &[
                ("jobid".into(), required(point, "jobid")?),
                ("knowledgeid".into(), task.chapter_id.to_string()),
                ("courseid".into(), task.course.id.to_string()),
                ("clazzid".into(), task.course.class_id.to_string()),
                ("jtoken".into(), required(point, "jtoken")?),
                ("_dc".into(), timestamp().to_string()),
            ],
        )?
        .json()?;
    ensure!(
        successful_receipt(&receipt),
        "document report lacks successful receipt"
    );
    thread::sleep(Duration::from_secs(1));
    Ok(receipt)
}

pub fn run_live(session: &Session, task: &Task) -> Result<Value> {
    ensure!(task.kind == TaskKind::Live, "task is not live");
    resume_reading(session, task)
}

pub fn run_article(session: &Session, task: &Task) -> Result<Value> {
    ensure!(task.kind == TaskKind::Article, "task is not article");
    resume_reading(session, task)
}

fn resume_reading(session: &Session, task: &Task) -> Result<Value> {
    ensure!(
        task.course.id > 0 && task.course.class_id > 0 && task.chapter_id > 0,
        "missing course task identifiers"
    );
    ensure!(
        task.point().is_some(),
        "reading attachment missing or ambiguous"
    );
    let html = session
        .get(
            "https://mooc1-api.chaoxing.com/knowledge/cards",
            &[
                ("clazzid".into(), task.course.class_id.to_string()),
                ("courseid".into(), task.course.id.to_string()),
                ("knowledgeid".into(), task.chapter_id.to_string()),
                ("num".into(), task.card_index.to_string()),
                ("isPhone".into(), "1".into()),
                ("control".into(), "true".into()),
                ("cpi".into(), task.course.cpi.to_string()),
            ],
        )?
        .text()?;
    let mut current = task.clone();
    current.attachment = crate::course::parse_attachment(&html)?;
    let point = current
        .point()
        .context("refreshed reading attachment missing or ambiguous")?;
    // The official reader marks completion through AttachmentSetting; opening it is not a receipt.
    let completed = current.already_complete();
    Ok(json!({
        "status": completed,
        "already_complete": completed,
        "action_required": !completed,
        "kind": task.kind,
        "isPassed": point.get("isPassed"),
        "job": point.get("job"),
        "study_url": format!(
            "https://mooc1.chaoxing.com/mycourse/studentstudy?chapterId={}&courseId={}&clazzid={}&cpi={}",
            task.chapter_id, task.course.id, task.course.class_id, task.course.cpi
        ),
        "message": if completed { "平台已确认任务完成或无需完成" } else { "请在课程页面完成直播观看或文章阅读，然后重新运行以读取平台完成状态" }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(responses: Vec<Value>) -> (Session, std::thread::JoinHandle<Vec<String>>) {
        raw_fixture(responses.into_iter().map(|v| v.to_string()).collect())
    }

    fn raw_fixture(responses: Vec<String>) -> (Session, std::thread::JoinHandle<Vec<String>>) {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(e) => panic!("fixture accept: {e}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                loop {
                    let mut bytes = [0; 2048];
                    let count = stream.read(&mut bytes).unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&bytes[..count]);
                    if request.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8(request).unwrap());
                let body = response;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
            requests
        });
        (Session::new(3, 0, Some(&url)).unwrap(), handle)
    }
    fn video_task() -> Task {
        Task {
            kind: TaskKind::Video,
            card_index: 0,
            chapter_id: 7,
            course: crate::course::Course {
                id: 1,
                class_id: 2,
                cpi: 3,
                key: 4,
                name: String::new(),
                teacher: String::new(),
            },
            property: json!({"objectid":"obj"}),
            attachment: json!({"defaults":{"fid":4},"attachments":[{"job":true,"jobid":"job", "otherInfo":"nodeId_7&courseId=1", "property":{"objectid":"obj"}}]}),
        }
    }
    #[test]
    fn local_video_receipt_and_query_protocol() {
        let (session, server) = fixture(vec![
            json!({"status":"success","dtoken":"tok","duration":1}),
            json!({"isPassed":false}),
            json!({"isPassed":true}),
        ]);
        let started = Instant::now();
        let receipt = run_video(&session, &video_task(), 9, 2.5, 1).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(400));
        assert_eq!(receipt["isPassed"], true);
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /ananas/status/obj?k=4&flag=normal&_dc="));
        assert!(requests[1].contains("otherInfo=nodeId_7&courseId=1&playingTime=0&duration=1"));
        assert!(requests[2].contains("playingTime=1&duration=1"));
    }
    #[test]
    fn video_duration_and_http_200_do_not_imply_completion() {
        for receipts in [
            vec![json!({})],
            vec![json!({"isPassed":false}), json!({"isPassed":false})],
        ] {
            let mut responses = vec![json!({"status":"success","dtoken":"tok","duration":1})];
            responses.extend(receipts);
            let (session, server) = fixture(responses);
            assert!(run_video(&session, &video_task(), 9, 1000.0, 1).is_err());
            server.join().unwrap();
        }
    }
    #[test]
    fn invalid_speed_and_completed_task_do_not_request() {
        let session = Session::new(1, 0, Some("http://127.0.0.1:1")).unwrap();
        let mut task = video_task();
        for speed in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(run_video(&session, &task, 9, speed, 1).is_err());
        }
        assert!(run_video(&session, &task, 9, 1.0, 0).is_err());
        task.attachment["attachments"][0]["isPassed"] = json!(true);
        assert_eq!(
            run_video(&session, &task, 9, 1.0, 1).unwrap()["already_complete"],
            true
        );
        task.kind = TaskKind::Document;
        assert_eq!(
            run_document(&session, &task).unwrap()["already_complete"],
            true
        );
    }
    #[test]
    fn local_document_receipt_and_ids() {
        let (session, server) = fixture(vec![json!({"status":true})]);
        let mut task = video_task();
        task.kind = TaskKind::Document;
        task.attachment["attachments"][0]["jtoken"] = json!("jt");
        assert_eq!(run_document(&session, &task).unwrap()["status"], true);
        let requests = server.join().unwrap();
        assert!(requests[0].contains("jobid=job&knowledgeid=7&courseid=1&clazzid=2&jtoken=jt"));
    }
    #[test]
    fn legacy_query_preserves_embedded_parameters() {
        assert_eq!(
            media_query(&[(
                "otherInfo".into(),
                "nodeId_1&courseId=2&title=中 文+%".into()
            )]),
            "otherInfo=nodeId_1&courseId=2&title=%E4%B8%AD+%E6%96%87%2B%25"
        );
    }
    #[test]
    fn signature_golden() {
        assert_eq!(
            video_signature(123, 456, "job7", "obj8", 9, 100),
            "6812df02eb4d4c276e39d08e097ce037"
        );
    }
    #[test]
    fn document_receipt_requires_positive_confirmation() {
        assert!(!successful_receipt(&json!({})));
        assert!(!successful_receipt(&json!({"status":false})));
        assert!(!successful_receipt(
            &json!({"status":true,"error":"failure"})
        ));
        assert!(successful_receipt(&json!({"status":true})));
    }

    #[test]
    fn reading_resume_uses_fresh_receipts_and_never_synthesizes_completion() {
        for kind in [TaskKind::Live, TaskKind::Article] {
            let key = if kind == TaskKind::Live {
                "vdoid"
            } else {
                "mid"
            };
            let mut task = video_task();
            task.kind = kind;
            task.property = json!({key: "resource1"});
            task.attachment = json!({"defaults":{},"attachments":[{"property":task.property,"job":true,"isPassed":true}]});
            for (fields, completed) in [
                (json!({"job":true,"isPassed":false}), false),
                (json!({"job":true,"isPassed":true}), true),
                (json!({"job":false}), true),
                (json!({"status":true}), false),
                (json!({"job":"false","isPassed":"true"}), false),
            ] {
                let mut point = fields;
                point["property"] = task.property.clone();
                let html = format!(
                    "<script>window.AttachmentSetting = {};</script>",
                    json!({"defaults":{},"attachments":[point]})
                );
                let (session, server) = raw_fixture(vec![html]);
                let receipt = if kind == TaskKind::Live {
                    run_live(&session, &task)
                } else {
                    run_article(&session, &task)
                }
                .unwrap();
                assert_eq!(receipt["status"], completed);
                assert_eq!(receipt["action_required"], !completed);
                assert!(receipt["study_url"].as_str().unwrap().starts_with("https://mooc1.chaoxing.com/mycourse/studentstudy?chapterId=7&courseId=1&clazzid=2&cpi=3"));
                let requests = server.join().unwrap();
                assert_eq!(requests.len(), 1);
                assert!(requests[0].starts_with("GET /knowledge/cards?clazzid=2&courseid=1&knowledgeid=7&num=0&isPhone=1&control=true&cpi=3 "));
            }
        }
    }

    #[test]
    fn reading_refresh_errors_and_missing_resources_stay_incomplete() {
        let mut task = video_task();
        task.kind = TaskKind::Article;
        task.property = json!({"mid":"resource1"});
        task.attachment =
            json!({"defaults":{},"attachments":[{"property":{"mid":"resource1"},"job":true}]});
        for html in [
            "<html>登录后查看</html>".to_string(),
            format!(
                "<script>window.AttachmentSetting={};</script>",
                json!({"defaults":{},"attachments":[{"property":{"mid":"other"},"isPassed":true}]})
            ),
            format!(
                "<script>window.AttachmentSetting={};</script>",
                json!({"defaults":{},"attachments":[{"property":{"mid":"resource1"},"isPassed":true},{"property":{"mid":"resource1"},"isPassed":true}]})
            ),
        ] {
            let (session, server) = raw_fixture(vec![html]);
            assert!(run_article(&session, &task).is_err());
            server.join().unwrap();
        }
        let session = Session::new(1, 0, Some("http://127.0.0.1:1")).unwrap();
        assert!(run_live(&session, &task).is_err());
        task.property = json!({"mid":"missing"});
        assert!(run_article(&session, &task).is_err());
    }
}
