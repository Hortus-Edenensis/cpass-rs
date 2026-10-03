use anyhow::{Context, Result, bail, ensure};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::transport::{Session, inf_enc_sign, timestamp};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Course {
    pub id: u64,
    pub class_id: u64,
    pub cpi: u64,
    pub key: u64,
    pub name: String,
    pub teacher: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub id: u64,
    pub label: String,
    pub name: String,
    pub total: usize,
    pub finished: usize,
    pub status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskKind {
    Video,
    Document,
    Work,
    Live,
    Article,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub kind: TaskKind,
    pub card_index: usize,
    pub chapter_id: u64,
    pub course: Course,
    pub attachment: Value,
    pub property: Value,
}

impl Task {
    pub fn point(&self) -> Option<&Value> {
        let points = self.attachment.get("attachments")?.as_array()?;
        let matches: Vec<_> = points
            .iter()
            .filter(|point| {
                if self.kind == TaskKind::Work {
                    let job = self.property.get("_jobid");
                    job.is_some() && job == point.get("jobid")
                } else {
                    let key = match self.kind {
                        TaskKind::Live => "vdoid",
                        TaskKind::Article => "mid",
                        _ => "objectid",
                    };
                    let object = self.property.get(key).filter(|v| scalar(v).is_some());
                    object.is_some() && object == point.get("property").and_then(|p| p.get(key))
                }
            })
            .collect();
        (matches.len() == 1).then(|| matches[0])
    }

    pub fn already_complete(&self) -> bool {
        if self.kind == TaskKind::Unknown {
            return false;
        }
        self.point().is_some_and(|p| {
            p.get("isPassed") == Some(&Value::Bool(true))
                || (self.kind != TaskKind::Video && p.get("job") == Some(&Value::Bool(false)))
        })
    }
}

pub(crate) fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn number(value: &Value) -> Result<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.parse().ok())
        .context("missing or invalid numeric identifier")
}

pub fn list(session: &Session) -> Result<Vec<Course>> {
    let data = session
        .get("https://mooc1-api.chaoxing.com/mycourse/backclazzdata", &[])?
        .json()?;
    ensure!(data["result"] == 1, "course list request was rejected");
    let mut courses = Vec::new();
    for item in data["channelList"]
        .as_array()
        .context("missing course list")?
    {
        let content = &item["content"];
        if content.get("course").is_none() {
            continue;
        }
        let course = &content["course"]["data"][0];
        courses.push(Course {
            id: number(&course["id"])?,
            class_id: number(&content["id"])?,
            cpi: number(&item["cpi"])?,
            key: number(&item["key"])?,
            name: course["name"]
                .as_str()
                .context("missing course name")?
                .to_owned(),
            teacher: course["teacherfactor"]
                .as_str()
                .unwrap_or("未知")
                .to_owned(),
        });
    }
    Ok(courses)
}

pub fn chapters(session: &Session, course: &Course, uid: u64) -> Result<Vec<Chapter>> {
    let data = session.get("https://mooc1-api.chaoxing.com/gas/clazz", &[
        ("id".into(), course.key.to_string()), ("personid".into(), course.cpi.to_string()),
        ("fields".into(), "id,bbsid,classscore,isstart,allowdownload,chatid,name,state,isfiled,visiblescore,begindate,coursesetting.fields(id,courseid,hiddencoursecover,coursefacecheck),course.fields(id,name,infocontent,objectid,app,bulletformat,mappingcourseid,imageurl,teacherfactor,jobcount,knowledge.fields(id,name,indexOrder,parentnodeid,status,layer,label,jobcount,begintime,endtime,attachment.fields(id,type,objectid,extension).type(video)))".into()),
        ("view".into(), "json".into()),
    ])?.json()?;
    let rows = data["data"][0]["course"]["data"][0]["knowledge"]["data"]
        .as_array()
        .context("missing chapters")?;
    let mut result = Vec::new();
    for row in rows {
        result.push(Chapter {
            id: number(&row["id"])?,
            label: row["label"]
                .as_str()
                .context("missing chapter label")?
                .to_owned(),
            name: row["name"]
                .as_str()
                .context("missing chapter name")?
                .trim()
                .to_owned(),
            total: 0,
            finished: 0,
            status: scalar(&row["status"]).unwrap_or_default(),
        });
    }
    result.sort_by_key(|c| {
        c.label
            .split('.')
            .map(|v| v.parse::<u64>().unwrap_or(u64::MAX))
            .collect::<Vec<_>>()
    });
    if result.is_empty() {
        return Ok(result);
    }
    let form = [
        ("view".into(), "json".into()),
        (
            "nodes".into(),
            result
                .iter()
                .map(|c| c.id.to_string())
                .collect::<Vec<_>>()
                .join(","),
        ),
        ("clazzid".into(), course.class_id.to_string()),
        ("time".into(), timestamp().to_string()),
        ("userid".into(), uid.to_string()),
        ("cpi".into(), course.cpi.to_string()),
        ("courseid".into(), course.id.to_string()),
    ]
    .into_iter()
    .collect();
    let states = session
        .post_form(
            "https://mooc1-api.chaoxing.com/job/myjobsnodesmap",
            &[],
            &form,
        )?
        .json()?;
    for chapter in &mut result {
        let state = &states[chapter.id.to_string()];
        let total = number(&state["totalcount"])?;
        let unfinished = number(&state["unfinishcount"])?;
        chapter.total = usize::try_from(if total == 0 { unfinished } else { total })?;
        chapter.finished = usize::try_from(number(&state["finishcount"])?)?;
        ensure!(
            chapter.finished <= chapter.total,
            "inconsistent chapter completion counts"
        );
    }
    Ok(result)
}

pub fn refresh_chapter(session: &Session, course: &Course, chapter_id: u64) -> Result<()> {
    session.get(
        "https://mooc1.chaoxing.com/mooc-ans/mycourse/studentstudyAjax",
        &[
            ("courseId".into(), course.id.to_string()),
            ("clazzid".into(), course.class_id.to_string()),
            ("chapterId".into(), chapter_id.to_string()),
            ("cpi".into(), course.cpi.to_string()),
            ("verificationcode".into(), String::new()),
            ("mooc2".into(), "1".into()),
        ],
    )?;
    Ok(())
}

pub fn chapter_tasks(
    session: &Session,
    course: &Course,
    chapter: &Chapter,
    _uid: u64,
) -> Result<Vec<Task>> {
    let params = inf_enc_sign(&[
        ("id".into(), chapter.id.to_string()), ("courseid".into(), course.id.to_string()),
        ("fields".into(), "id,parentnodeid,indexorder,label,layer,name,begintime,createtime,lastmodifytime,status,jobUnfinishedCount,clickcount,openlock,card.fields(id,knowledgeid,title,knowledgeTitile,description,cardorder).contentcard(all)".into()),
        ("view".into(), "json".into()), ("token".into(), "4faa8662c59590c6f43ae9fe5b002b42".into()),
        ("_time".into(), timestamp().to_string()),
    ]);
    let data = session
        .get("https://mooc1-api.chaoxing.com/gas/knowledge", &params)?
        .json()?;
    let cards = data["data"][0]["card"]["data"]
        .as_array()
        .context("missing chapter cards")?;
    let mut tasks = Vec::new();
    for (card_index, card) in cards.iter().enumerate() {
        let frames = parse_frames(card["description"].as_str().unwrap_or_default());
        if frames.is_empty() {
            continue;
        }
        let attachment = if frames.iter().any(|(kind, _)| *kind != TaskKind::Unknown) {
            session
                .get(
                    "https://mooc1-api.chaoxing.com/knowledge/cards",
                    &[
                        ("clazzid".into(), course.class_id.to_string()),
                        ("courseid".into(), course.id.to_string()),
                        ("knowledgeid".into(), chapter.id.to_string()),
                        ("num".into(), card_index.to_string()),
                        ("isPhone".into(), "1".into()),
                        ("control".into(), "true".into()),
                        ("cpi".into(), course.cpi.to_string()),
                    ],
                )
                .and_then(|response| response.text())
                .map_err(|_| "card attachment request failed")
                .and_then(|html| {
                    parse_attachment(&html).map_err(|_| "card attachment parsing failed")
                })
        } else {
            Ok(Value::Null)
        };
        for (kind, property) in frames {
            let (kind, property, attachment) = match &attachment {
                Ok(attachment) => (kind, property, attachment.clone()),
                Err(reason) => (
                    TaskKind::Unknown,
                    serde_json::json!({"frame":property,"task_kind":kind,"parse_error":reason}),
                    Value::Null,
                ),
            };
            tasks.push(Task {
                kind,
                card_index,
                chapter_id: chapter.id,
                course: course.clone(),
                attachment,
                property,
            });
        }
    }
    Ok(tasks)
}

fn parse_frames(html: &str) -> Vec<(TaskKind, Value)> {
    let fragment = Html::parse_fragment(html);
    let selector = Selector::parse("iframe").expect("static selector");
    fragment
        .select(&selector)
        .map(|frame| {
            let kind = match frame.value().attr("module") {
                Some("insertvideo") => TaskKind::Video,
                Some("insertdoc") => TaskKind::Document,
                Some("work") => TaskKind::Work,
                Some("insertlive") => TaskKind::Live,
                Some("insertread") => TaskKind::Article,
                _ => TaskKind::Unknown,
            };
            let property = frame
                .value()
                .attr("data")
                .and_then(|v| serde_json::from_str::<Value>(v).ok())
                .unwrap_or(Value::Null);
            (
                if property.is_object() {
                    kind
                } else {
                    TaskKind::Unknown
                },
                property,
            )
        })
        .collect()
}

pub(crate) fn parse_attachment(html: &str) -> Result<Value> {
    let dom = Html::parse_document(html);
    let selector = Selector::parse("script").expect("static selector");
    let assignment = regex::Regex::new(r"window\.AttachmentSetting\s*=").expect("static regex");
    for script in dom.select(&selector) {
        let text = script.inner_html();
        if let Some(found) = assignment.find(&text) {
            let mut stream = serde_json::Deserializer::from_str(text[found.end()..].trim_start())
                .into_iter::<Value>();
            let value = stream.next().context("empty AttachmentSetting")??;
            ensure!(
                value["attachments"].is_array() && value["defaults"].is_object(),
                "invalid AttachmentSetting"
            );
            return Ok(value);
        }
    }
    if html.contains("章节未开放") {
        bail!("章节未开放");
    }
    bail!("missing AttachmentSetting")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn unknown_and_malformed_frames_are_preserved() {
        let frames = parse_frames(
            r#"<iframe data='{}'></iframe><iframe module='newtype' data='{}'></iframe><iframe module='insertvideo' data='broken'></iframe><iframe module='insertdoc' data='{&quot;objectid&quot;:&quot;x&quot;}'></iframe>"#,
        );
        assert_eq!(frames.len(), 4);
        assert!(
            frames[..3]
                .iter()
                .all(|(kind, _)| *kind == TaskKind::Unknown)
        );
        assert_eq!(frames[3], (TaskKind::Document, json!({"objectid":"x"})));
    }
    #[test]
    fn multiline_attachment_preserves_semicolons_and_script_suffix() {
        let parsed = parse_attachment(
            r#"<script>window.AttachmentSetting = {
            "attachments": [{"jobid":"1;2"}], "defaults": {"fid":1}
        }; window.other = true;</script>"#,
        )
        .unwrap();
        assert_eq!(parsed["attachments"][0]["jobid"], "1;2");
        assert!(parse_attachment("<p>章节未开放！</p>").is_err());
    }
    #[test]
    fn no_job_and_passed_skip_but_missing_attachment_does_not() {
        let mut task = Task {
            kind: TaskKind::Document,
            card_index: 0,
            chapter_id: 1,
            course: Course {
                id: 1,
                class_id: 2,
                cpi: 3,
                key: 4,
                name: String::new(),
                teacher: String::new(),
            },
            property: json!({"objectid":"obj"}),
            attachment: json!({"attachments":[{"property":{"objectid":"obj"},"job":false}]}),
        };
        assert!(task.already_complete());
        task.attachment["attachments"][0]["job"] = json!(true);
        assert!(!task.already_complete());
        task.attachment["attachments"][0]["isPassed"] = json!(true);
        assert!(task.already_complete());
        task.property["objectid"] = json!("missing");
        assert!(!task.already_complete());
    }

    #[test]
    fn live_and_article_frames_use_their_resource_identifiers() {
        let frames = parse_frames(
            r#"<iframe module="insertlive" data='{"vdoid":"live1"}'></iframe><iframe module="insertread" data='{"mid":"read1"}'></iframe><iframe module="article" data='{"mid":"read1"}'></iframe>"#,
        );
        assert_eq!(frames[0].0, TaskKind::Live);
        assert_eq!(frames[1].0, TaskKind::Article);
        assert_eq!(frames[2].0, TaskKind::Unknown);
        for (kind, property) in &frames[..2] {
            let mut task = Task {
                kind: *kind,
                card_index: 0,
                chapter_id: 1,
                course: Course {
                    id: 1,
                    class_id: 2,
                    cpi: 3,
                    key: 4,
                    name: String::new(),
                    teacher: String::new(),
                },
                property: property.clone(),
                attachment: json!({"attachments":[{"property": property,"isPassed":true}]}),
            };
            assert!(task.already_complete());
            let duplicate = task.attachment["attachments"][0].clone();
            task.attachment["attachments"]
                .as_array_mut()
                .unwrap()
                .push(duplicate);
            assert!(task.point().is_none());
            assert!(!task.already_complete());
            task.property = json!({"vdoid":null,"mid":null});
            task.attachment =
                json!({"attachments":[{"property":{"vdoid":null,"mid":null},"isPassed":true}]});
            assert!(task.point().is_none());
        }
    }

    #[test]
    fn video_requires_explicit_passed_receipt_even_without_active_job() {
        let mut task = Task {
            kind: TaskKind::Video,
            card_index: 0,
            chapter_id: 1,
            course: Course {
                id: 1,
                class_id: 2,
                cpi: 3,
                key: 4,
                name: String::new(),
                teacher: String::new(),
            },
            property: json!({"objectid":"obj"}),
            attachment: json!({"attachments":[{"property":{"objectid":"obj"},"job":false}]}),
        };
        assert!(!task.already_complete());
        for passed in [json!(false), json!("true"), json!(1), Value::Null] {
            task.attachment["attachments"][0]["isPassed"] = passed;
            assert!(!task.already_complete());
        }
        task.attachment["attachments"][0]["isPassed"] = json!(true);
        assert!(task.already_complete());
        task.attachment["attachments"][0]["isPassed"] = json!(false);
        task.kind = TaskKind::Document;
        assert!(task.already_complete());
        task.kind = TaskKind::Work;
        task.property = json!({"_jobid":"job1"});
        task.attachment["attachments"][0]["jobid"] = json!("job1");
        assert!(task.already_complete());
    }

    #[test]
    fn bad_card_preserves_previous_and_following_tasks() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
            time::{Duration, Instant},
        };
        for (bad_status, expected_reason) in [
            ("200 OK", "card attachment parsing failed"),
            ("503 Service Unavailable", "card attachment request failed"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let session = Session::new(
                2,
                0,
                Some(&format!("http://{}", listener.local_addr().unwrap())),
            )
            .unwrap();
            let cards = (0..3).map(|i| json!({"description":format!("<iframe module='insertvideo' data='{{\"objectid\":\"obj{i}\"}}'></iframe>")})).collect::<Vec<_>>();
            let good = |i| {
                format!(
                    "<script>window.AttachmentSetting={};</script>",
                    json!({"defaults":{},"attachments":[{"property":{"objectid":format!("obj{i}")},"isPassed":true}]})
                )
            };
            let replies = [
                (
                    "200 OK",
                    json!({"data":[{"card":{"data":cards}}]}).to_string(),
                ),
                ("200 OK", good(0)),
                (bad_status, "<html>private-response-marker</html>".into()),
                ("200 OK", good(2)),
            ];
            let server = thread::spawn(move || {
                let mut requests = Vec::new();
                for (status, body) in replies {
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
                            Err(e) => panic!("fixture accept failed: {e}"),
                        }
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = Vec::new();
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        let mut buf = [0; 2048];
                        let n = stream.read(&mut buf).unwrap();
                        assert!(n > 0);
                        request.extend_from_slice(&buf[..n]);
                    }
                    requests.push(String::from_utf8(request).unwrap());
                    write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
                requests
            });
            let course = Course {
                id: 1,
                class_id: 2,
                cpi: 3,
                key: 4,
                name: String::new(),
                teacher: String::new(),
            };
            let chapter = Chapter {
                id: 7,
                label: "1".into(),
                name: String::new(),
                total: 3,
                finished: 0,
                status: String::new(),
            };
            let tasks = chapter_tasks(&session, &course, &chapter, 9).unwrap();
            let requests = server.join().unwrap();
            assert_eq!(tasks.len(), 3);
            assert_eq!(
                tasks.iter().map(|t| t.kind).collect::<Vec<_>>(),
                [TaskKind::Video, TaskKind::Unknown, TaskKind::Video]
            );
            assert!(tasks[0].already_complete() && tasks[2].already_complete());
            assert!(!tasks[1].already_complete());
            for (index, task) in tasks.iter().enumerate() {
                assert_eq!(task.card_index, index);
                assert_eq!(task.chapter_id, 7);
                assert_eq!(task.course.id, 1);
            }
            assert_eq!(tasks[1].property["frame"], json!({"objectid":"obj1"}));
            assert_eq!(tasks[1].property["parse_error"], expected_reason);
            assert_eq!(tasks[1].property["task_kind"], "Video");
            assert!(
                !serde_json::to_string(&tasks)
                    .unwrap()
                    .contains("private-response-marker")
            );
            assert_eq!(requests.len(), 4);
            assert!(requests[0].starts_with("GET /gas/knowledge?"));
            for (index, request) in requests[1..].iter().enumerate() {
                assert!(request.starts_with("GET /knowledge/cards?"));
                assert!(request.contains(&format!("&num={index}&")));
            }
        }
    }
}
