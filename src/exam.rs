use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use reqwest::Url;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::course::Course;
use crate::model::Question;
use crate::questions::{PageKind, ParsedPaper, exam_form, parse_page, valid_answer};
use crate::transport::{Params, Session, exam_signature, imei};

const LIST: &str = "https://mooc1-api.chaoxing.com/exam/phone/task-list";
const COVER: &str = "https://mooc1-api.chaoxing.com/exam-ans/exam/phone/task-exam";
const START: &str = "https://mooc1-api.chaoxing.com/exam-ans/exam/phone/start";
const QUESTION: &str = "https://mooc1-api.chaoxing.com/exam-ans/exam/test/reVersionTestStartNew";
const PREVIEW: &str = "https://mooc1-api.chaoxing.com/exam-ans/exam/phone/preview";
const SHEET: &str = "https://mooc1-api.chaoxing.com/exam-ans/exam/phone/loadAnswerStatic";
const COMMIT: &str = "https://mooc1.chaoxing.com/exam-ans/exam/test/reVersionSubmitTestNew";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExamInfo {
    pub id: u64,
    pub course_id: u64,
    pub class_id: u64,
    pub cpi: u64,
    pub enc_task: String,
    pub name: String,
    pub status: String,
    pub expire_time: String,
}

fn select(selector: &str) -> Selector {
    Selector::parse(selector).expect("static selector")
}

fn input(html: &Html, id: &str) -> Result<String> {
    html.select(&select(&format!("input[id='{id}']")))
        .next()
        .and_then(|node| node.value().attr("value"))
        .map(str::to_owned)
        .with_context(|| format!("考试表单缺少 {id}"))
}

fn flag(value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" => Ok(false),
        "1" | "true" => Ok(true),
        _ => bail!("考试要求标志无效"),
    }
}

fn number(value: &str, field: &str) -> Result<u64> {
    value
        .parse()
        .with_context(|| format!("考试字段 {field} 不是非负整数"))
}

fn page_error(text: &str) -> Result<Option<String>> {
    let html = Html::parse_document(text);
    Ok(html
        .select(&select("p.blankTips, li.msg, h2.color6.fs36.textCenter"))
        .next()
        .map(|node| node.text().collect::<String>().trim().to_owned()))
}

fn confirm(value: &Value) -> Result<()> {
    ensure!(
        value.get("status").and_then(Value::as_str) == Some("success"),
        "考试提交或交卷回执未确认成功"
    );
    Ok(())
}

fn reconcile_sheet(paper: &mut ParsedPaper, sheet: &Value) -> Result<usize> {
    let entries = sheet.as_array().context("考试答题卡格式无效")?;
    let indices = entries
        .iter()
        .map(|entry| {
            entry["index"]
                .as_u64()
                .and_then(|index| usize::try_from(index).ok())
                .context("考试答题卡题号无效")
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let count = indices.len();
    ensure!(
        count > 0 && entries.len() == count && indices == (0..count).collect(),
        "考试答题卡题号缺失或重复"
    );
    if paper
        .declared_count
        .is_some_and(|declared| declared != count)
    {
        paper
            .parse_errors
            .insert(0, "考试声明总数与答题卡冲突".into());
    }
    for index in 0..count {
        if !paper.indices.contains(&index) && !paper.parse_errors.contains_key(&index) {
            paper
                .parse_errors
                .insert(index, "考试预览缺少答题卡中的题目".into());
        }
    }
    for index in &paper.indices {
        if *index >= count {
            paper
                .parse_errors
                .insert(*index, "考试预览题目超出答题卡边界".into());
        }
    }
    paper.declared_count = Some(count);
    Ok(count)
}

pub fn list(session: &Session, course: &Course) -> Result<Vec<ExamInfo>> {
    let query = vec![
        ("courseId".into(), course.id.to_string()),
        ("classId".into(), course.class_id.to_string()),
        ("cpi".into(), course.cpi.to_string()),
    ];
    let text = session.get(LIST, &query)?.text()?;
    if page_error(&text)?.is_some() {
        bail!("考试列表不可用或访问受限");
    }
    let html = Html::parse_document(&text);
    let mut result = Vec::new();
    let mut ids = BTreeSet::new();
    for item in html.select(&select("ul.nav li[data]")) {
        let link = item.value().attr("data").context("考试列表缺少入口")?;
        let link = html_escape::decode_html_entities(link);
        let url = Url::parse(LIST)?.join(&link)?;
        let query: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
        let id = number(
            query.get("taskrefId").context("考试入口缺少 taskrefId")?,
            "taskrefId",
        )?;
        ensure!(id > 0 && ids.insert(id), "考试 ID 缺失或重复");
        let name = item
            .select(&select("p"))
            .next()
            .context("考试列表缺少标题")?
            .text()
            .collect::<String>();
        let status = item
            .select(&select("span:not(.fr)"))
            .next()
            .context("考试列表缺少状态")?
            .text()
            .collect::<String>();
        let expire_time = item
            .select(&select("span.fr"))
            .next()
            .map(|node| node.text().collect::<String>())
            .unwrap_or_default();
        result.push(ExamInfo {
            id,
            course_id: course.id,
            class_id: course.class_id,
            cpi: course.cpi,
            enc_task: query
                .get("enc_task")
                .context("考试入口缺少 enc_task")?
                .clone(),
            name: name.trim().into(),
            status: status.trim().into(),
            expire_time: expire_time.trim().into(),
        });
    }
    Ok(result)
}

pub struct Exam {
    session: Session,
    pub info: ExamInfo,
    uid: u64,
    answer_id: u64,
    need_code: bool,
    need_face: bool,
    need_captcha: bool,
    captcha_id: String,
    captcha_validate: String,
    face_key: String,
    face_result: Value,
    metadata_loaded: bool,
    started: bool,
    enc: String,
    enc_remain_time: u64,
    remain_time: u64,
    last_update_time: u64,
    expected: BTreeMap<usize, u64>,
    expected_count: Option<usize>,
    seen: BTreeMap<usize, u64>,
    confirmed: BTreeSet<u64>,
    parse_errors: BTreeMap<usize, String>,
}

impl Exam {
    pub fn new(session: Session, info: ExamInfo, uid: u64) -> Self {
        Self {
            session,
            info,
            uid,
            answer_id: 0,
            need_code: false,
            need_face: false,
            need_captcha: false,
            captcha_id: String::new(),
            captcha_validate: String::new(),
            face_key: String::new(),
            face_result: Value::Null,
            metadata_loaded: false,
            started: false,
            enc: String::new(),
            enc_remain_time: 0,
            remain_time: 0,
            last_update_time: 0,
            expected: BTreeMap::new(),
            expected_count: None,
            seen: BTreeMap::new(),
            confirmed: BTreeSet::new(),
            parse_errors: BTreeMap::new(),
        }
    }

    pub fn set_challenge_data(
        &mut self,
        captcha_validate: Option<String>,
        face_key: Option<String>,
        face_result: Option<Value>,
    ) {
        if let Some(value) = captcha_validate {
            self.captcha_validate = value;
        }
        if let Some(value) = face_key {
            self.face_key = value;
        }
        if let Some(value) = face_result {
            self.face_result = value;
        }
    }

    pub fn parse_errors(&self) -> &BTreeMap<usize, String> {
        &self.parse_errors
    }

    pub fn expected_count(&self) -> Option<usize> {
        self.expected_count
    }

    pub fn metadata(&mut self) -> Result<Value> {
        let query = vec![
            ("redo".into(), "1".into()),
            ("taskrefId".into(), self.info.id.to_string()),
            ("courseId".into(), self.info.course_id.to_string()),
            ("classId".into(), self.info.class_id.to_string()),
            ("userId".into(), self.uid.to_string()),
            ("role".into(), "".into()),
            ("source".into(), "0".into()),
            ("enc_task".into(), self.info.enc_task.clone()),
            ("cpi".into(), self.info.cpi.to_string()),
            ("vx".into(), "0".into()),
            ("examsignal".into(), "1".into()),
        ];
        let response = self.session.get_no_redirect(COVER, &query)?;
        if response.status == 302 {
            let location = response
                .headers
                .get("location")
                .or_else(|| response.headers.get("Location"))
                .context("考试重定向缺少 Location")?;
            if Url::parse(COVER)?.join(location)?.path() == "/exam-ans/exam/phone/look" {
                bail!("考试已经完成");
            }
            bail!("考试入口发生未识别重定向");
        }
        ensure!(response.status == 200, "考试入口 HTTP 状态无效");
        let text = response.text()?;
        if page_error(&text)?.is_some() {
            bail!("考试尚未开放、访问受限或前置任务未完成");
        }
        let html = Html::parse_document(&text);
        self.answer_id = number(&input(&html, "testUserRelationId")?, "testUserRelationId")?;
        ensure!(self.answer_id > 0, "考试答卷 ID 无效");
        input(&html, "monitorEnc")?;
        if let Some(title) = html.select(&select("span.overHidden2")).next() {
            self.info.name = title.text().collect::<String>().trim().to_owned();
        }
        let need_code = Regex::new(r"\bneedcode\s*=\s*(\d+)\s*;")?
            .captures(&text)
            .context("考试封面缺少 needcode")?
            .get(1)
            .unwrap()
            .as_str();
        self.need_code = flag(need_code)?;
        self.need_face = flag(&input(&html, "faceRecognitionCompare")?)?;
        self.need_captcha = flag(&input(&html, "captchaCheck")?)?;
        self.captcha_id = input(&html, "captchaCaptchaId")?;
        self.metadata_loaded = true;
        Ok(
            json!({"id": self.info.id, "name": self.info.name, "need_code": self.need_code,
            "need_face": self.need_face, "need_captcha": self.need_captcha,
            "captcha_id": self.captcha_id}),
        )
    }

    fn check_challenge(&self) -> Result<()> {
        if self.need_face && (self.face_key.is_empty() || !self.face_result.is_object()) {
            bail!("action-required: 考试要求真实人脸验证，请完成客户端验证并提供有效验证回执");
        }
        if self.need_captcha && self.captcha_validate.is_empty() {
            bail!("action-required: 考试要求滑块验证码，请完成验证并提供 validate");
        }
        Ok(())
    }

    pub fn start(&mut self, code: Option<&str>) -> Result<()> {
        ensure!(self.metadata_loaded, "请先获取考试封面");
        self.check_challenge()?;
        ensure!(
            !self.need_code || code.is_some_and(|value| !value.is_empty()),
            "action-required: 考试需要考试码"
        );
        let query = vec![
            ("courseId".into(), self.info.course_id.to_string()),
            ("classId".into(), self.info.class_id.to_string()),
            ("examId".into(), self.info.id.to_string()),
            ("source".into(), "0".into()),
            ("examAnswerId".into(), self.answer_id.to_string()),
            ("cpi".into(), self.info.cpi.to_string()),
            ("keyboardDisplayRequiresUserAction".into(), "1".into()),
            ("imei".into(), imei()),
            ("faceDetection".into(), u8::from(self.need_face).to_string()),
            (
                "facekey".into(),
                if self.need_face {
                    self.face_key.clone()
                } else {
                    String::new()
                },
            ),
            (
                "faceDetectionResult".into(),
                if self.need_face {
                    serde_json::to_string(&self.face_result)?
                } else {
                    String::new()
                },
            ),
            ("captchavalidate".into(), self.captcha_validate.clone()),
            ("jt".into(), "0".into()),
            ("code".into(), code.unwrap_or_default().into()),
        ];
        let response = self.session.get_no_redirect(START, &query)?;
        ensure!(
            response.status == 302,
            "考试开始未确认成功，检查考试码与验证要求"
        );
        let location = response
            .headers
            .get("location")
            .or_else(|| response.headers.get("Location"))
            .context("考试开始缺少 Location")?;
        let target = Url::parse(START)?.join(location)?;
        self.enc = target
            .query_pairs()
            .find(|(key, _)| key == "enc")
            .map(|(_, value)| value.into_owned())
            .context("考试开始缺少 enc")?;
        ensure!(!self.enc.is_empty(), "考试开始 enc 为空");
        self.started = true;
        self.seen.clear();
        self.confirmed.clear();
        self.parse_errors.clear();
        self.expected.clear();
        self.expected_count = None;
        Ok(())
    }

    fn common_query(&self) -> Params {
        vec![
            ("courseId".into(), self.info.course_id.to_string()),
            ("classId".into(), self.info.class_id.to_string()),
            ("source".into(), "0".into()),
            ("cpi".into(), self.info.cpi.to_string()),
            ("imei".into(), imei()),
            ("enc".into(), self.enc.clone()),
            ("remainTimeParam".into(), self.enc_remain_time.to_string()),
            (
                "relationAnswerLastUpdateTime".into(),
                self.last_update_time.to_string(),
            ),
        ]
    }

    fn update_page(&mut self, paper: &ParsedPaper) -> Result<()> {
        let get = |key: &str| {
            paper
                .form
                .get(key)
                .with_context(|| format!("考试页缺少 {key}"))
        };
        let enc = get("enc")?.clone();
        ensure!(!enc.is_empty(), "考试页 enc 为空");
        let enc_remain_time = number(get("encRemainTime")?, "encRemainTime")?;
        let remain_time = number(get("remainTime")?, "remainTime")?;
        let last_update_time = number(get("encLastUpdateTime")?, "encLastUpdateTime")?;
        self.enc = enc;
        self.enc_remain_time = enc_remain_time;
        self.remain_time = remain_time;
        self.last_update_time = last_update_time;
        Ok(())
    }

    pub fn preview(&mut self) -> Result<ParsedPaper> {
        ensure!(self.started, "考试尚未开始");
        self.expected_count = None;
        self.expected.clear();
        self.seen.clear();
        self.confirmed.clear();
        let mut query = self.common_query();
        query.extend([
            ("start".into(), "0".into()),
            ("examRelationId".into(), self.info.id.to_string()),
            ("examRelationAnswerId".into(), self.answer_id.to_string()),
            ("monitorStatus".into(), "0".into()),
            ("monitorOp".into(), "-1".into()),
        ]);
        let text = self.session.get(PREVIEW, &query)?.text()?;
        if page_error(&text)?.is_some() {
            bail!("考试预览不可用或访问受限");
        }
        let mut paper = parse_page(&text, PageKind::Exam)?;
        self.update_page(&paper)?;
        let count = reconcile_sheet(&mut paper, &self.answer_sheet()?)?;
        self.parse_errors = paper.parse_errors.clone();
        self.expected = paper
            .indices
            .iter()
            .copied()
            .zip(paper.questions.iter().map(|question| question.id))
            .collect();
        self.expected_count = Some(count);
        ensure!(
            self.expected
                .values()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                == self.expected.len(),
            "考试预览题目 ID 重复"
        );
        Ok(paper)
    }

    pub fn fetch(&mut self, index: usize) -> Result<Option<ParsedPaper>> {
        ensure!(self.started, "考试尚未开始");
        if self.expected_count.is_some_and(|count| index >= count) {
            return Ok(None);
        }
        if let Some(id) = self.seen.remove(&index) {
            self.confirmed.remove(&id);
        }
        let mut query = self.common_query();
        query.extend([
            ("tId".into(), self.info.id.to_string()),
            ("id".into(), self.answer_id.to_string()),
            ("p".into(), "1".into()),
            ("isphone".into(), "true".into()),
            (
                "tag".into(),
                u8::from(self.enc_remain_time == 0).to_string(),
            ),
            ("start".into(), index.to_string()),
            ("keyboardDisplayRequiresUserAction".into(), "1".into()),
            ("monitorStatus".into(), "0".into()),
            ("monitorOp".into(), "-1".into()),
        ]);
        let text = self.session.get(QUESTION, &query)?.text()?;
        if let Some(error) = page_error(&text)? {
            if error == "无效参数！" {
                self.parse_errors
                    .insert(index, "考试在可信题目边界之前提前结束".into());
                bail!("考试题目提前结束，禁止交卷");
            }
            bail!("考试题目不可用或访问受限");
        }
        let mut paper = parse_page(&text, PageKind::Exam)?;
        self.update_page(&paper)?;
        if !paper.parse_errors.is_empty() || paper.questions.len() != 1 {
            let error = "考试单题页面未完整解析".to_string();
            self.parse_errors.insert(index, error.clone());
            paper.parse_errors = BTreeMap::from([(index, error)]);
        } else {
            let question = &paper.questions[0];
            if self
                .expected
                .get(&index)
                .is_some_and(|id| *id != question.id)
                || self
                    .seen
                    .iter()
                    .any(|(other, id)| *other != index && *id == question.id)
            {
                let error = "考试题目 ID 与预览不符或重复".to_string();
                self.parse_errors.insert(index, error.clone());
                paper.parse_errors.insert(index, error);
            } else {
                self.seen.insert(index, question.id);
                self.parse_errors.remove(&index);
                if valid_answer(question) {
                    self.confirmed.insert(question.id);
                } else {
                    self.confirmed.remove(&question.id);
                }
            }
        }
        paper.indices = vec![index; paper.questions.len()];
        Ok(Some(paper))
    }

    pub fn answer_sheet(&self) -> Result<Value> {
        ensure!(self.started, "考试尚未开始");
        let mut query = self.common_query();
        query.extend([
            ("start".into(), "0".into()),
            ("examRelationId".into(), self.info.id.to_string()),
            ("examRelationAnswerId".into(), self.answer_id.to_string()),
        ]);
        let text = self.session.get(SHEET, &query)?.text()?;
        if page_error(&text)?.is_some() {
            bail!("考试答题卡不可用");
        }
        let html = Html::parse_document(&text);
        let mut entries = Vec::new();
        for item in html.select(&select("ul li[data]")) {
            let index = item
                .value()
                .attr("data")
                .unwrap()
                .parse::<usize>()
                .context("答题卡题号无效")?;
            entries.push(json!({"index": index, "answered": item.value().classes().any(|class| class == "complated")}));
        }
        Ok(Value::Array(entries))
    }

    fn request(
        &self,
        index: usize,
        question: Option<&Question>,
        final_submit: bool,
    ) -> Result<(Params, BTreeMap<String, String>)> {
        ensure!(self.started, "考试尚未开始");
        if final_submit {
            self.validate_final()?;
        }
        if let Some(question) = question {
            ensure!(valid_answer(question), "答案无效或未完成");
            ensure!(
                self.seen.get(&index) == Some(&question.id),
                "提交题目必须与当前已读取页一致"
            );
            ensure!(!self.parse_errors.contains_key(&index), "题目解析未完成");
        } else {
            ensure!(final_submit, "缺少待提交题目");
        }
        let temp_save = if final_submit { "false" } else { "true" };
        let qid = question.map_or(0, |question| question.id);
        let mut query = vec![
            ("classId".into(), self.info.class_id.to_string()),
            ("courseId".into(), self.info.course_id.to_string()),
            ("cpi".into(), self.info.cpi.to_string()),
            ("testPaperId".into(), self.info.id.to_string()),
            ("testUserRelationId".into(), self.answer_id.to_string()),
            ("tempSave".into(), temp_save.into()),
        ];
        query.extend(exam_signature(self.uid, qid));
        query.extend([
            (
                "qid".into(),
                if qid == 0 {
                    String::new()
                } else {
                    qid.to_string()
                },
            ),
            ("version".into(), "1".into()),
        ]);
        let mut form = vec![
            ("courseId".into(), self.info.course_id.to_string()),
            ("testPaperId".into(), self.info.id.to_string()),
            ("testUserRelationId".into(), self.answer_id.to_string()),
            ("classId".into(), self.info.class_id.to_string()),
            ("type".into(), "0".into()),
            ("isphone".into(), "true".into()),
            ("imei".into(), imei()),
            ("subCount".into(), "".into()),
            ("remainTime".into(), self.remain_time.to_string()),
            ("tempSave".into(), temp_save.into()),
            ("timeOver".into(), "false".into()),
            ("encRemainTime".into(), self.enc_remain_time.to_string()),
            (
                "encLastUpdateTime".into(),
                self.last_update_time.to_string(),
            ),
            ("enc".into(), self.enc.clone()),
            ("userId".into(), self.uid.to_string()),
            ("source".into(), "0".into()),
            ("start".into(), index.to_string()),
            ("enterPageTime".into(), self.last_update_time.to_string()),
            ("monitorforcesubmit".into(), "0".into()),
            ("answeredView".into(), "0".into()),
            ("exitdtime".into(), "0".into()),
        ];
        if let Some(question) = question {
            form.extend(exam_form(question)?);
        }
        Ok((query, form.into_iter().collect()))
    }

    fn validate_final(&self) -> Result<()> {
        let count = self.expected_count.context("考试尚未完整预览，不能交卷")?;
        ensure!(
            count > 0 && self.expected.len() == count,
            "考试预览缺题，不能交卷"
        );
        ensure!(
            self.parse_errors.is_empty(),
            "考试存在解析失败题目，不能交卷"
        );
        ensure!(self.seen == self.expected, "考试题目尚未全部读取并核对");
        ensure!(
            self.expected.values().all(|id| self.confirmed.contains(id)),
            "考试存在未确认答案，不能交卷"
        );
        Ok(())
    }

    pub fn submit(&mut self, index: usize, question: &Question) -> Result<Value> {
        let (query, form) = self.request(index, Some(question), false)?;
        let value = self.session.post_form(COMMIT, &query, &form)?.json()?;
        self.accept_submission(&value, question.id)?;
        Ok(value)
    }

    fn accept_submission(&mut self, value: &Value, question_id: u64) -> Result<()> {
        confirm(value)?;
        let data = value
            .get("data")
            .and_then(Value::as_str)
            .context("考试单题回执缺少会话参数")?;
        let values: Vec<_> = data.split('|').collect();
        ensure!(
            values.len() == 3 && !values[2].is_empty(),
            "考试单题回执会话参数无效"
        );
        let last_update = number(values[0], "encLastUpdateTime")?;
        let remain = number(values[1], "encRemainTime")?;
        self.last_update_time = last_update;
        self.enc_remain_time = remain;
        self.enc = values[2].into();
        self.confirmed.insert(question_id);
        Ok(())
    }

    pub fn final_submit(&mut self) -> Result<Value> {
        let (query, form) = self.request(0, None, true)?;
        let value = self.session.post_form(COMMIT, &query, &form)?.json()?;
        confirm(&value)?;
        self.started = false;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exam() -> Exam {
        let session = Session::new(5, 0, Some("http://127.0.0.1:1")).unwrap();
        Exam::new(
            session,
            ExamInfo {
                id: 10,
                course_id: 20,
                class_id: 30,
                cpi: 1,
                enc_task: "test".into(),
                name: "test".into(),
                status: "未开始".into(),
                expire_time: "".into(),
            },
            40,
        )
    }

    #[test]
    fn flags_and_receipts_do_not_coerce_strings() {
        assert!(!flag("0").unwrap());
        assert!(flag("1").unwrap());
        assert!(flag("unknown").is_err());
        assert!(confirm(&json!({"status":"success"})).is_ok());
        assert!(confirm(&json!({"status":true})).is_err());
    }

    #[test]
    fn sheet_establishes_preview_count_and_detects_missing_pages() {
        let mut paper = ParsedPaper {
            questions: vec![],
            indices: vec![],
            parse_errors: BTreeMap::new(),
            form: BTreeMap::new(),
            declared_count: None,
            title: String::new(),
        };
        assert_eq!(
            reconcile_sheet(&mut paper, &json!([{"index":0},{"index":1}])).unwrap(),
            2
        );
        assert_eq!(paper.declared_count, Some(2));
        assert_eq!(paper.parse_errors.len(), 2);
        assert!(reconcile_sheet(&mut paper, &json!([{"index":0},{"index":0}])).is_err());
        assert!(reconcile_sheet(&mut paper, &json!([{"index":0},{"index":2}])).is_err());
    }

    #[test]
    fn unpreviewed_missing_and_unconfirmed_questions_block_final() {
        let mut exam = exam();
        assert!(exam.validate_final().is_err());
        exam.expected_count = Some(2);
        exam.expected.insert(0, 1);
        exam.confirmed.insert(1);
        assert!(exam.validate_final().is_err());
        exam.expected.insert(1, 2);
        assert!(exam.validate_final().is_err());
        exam.confirmed.insert(2);
        assert!(exam.validate_final().is_err());
        exam.seen = exam.expected.clone();
        assert!(exam.validate_final().is_ok());
        exam.parse_errors.insert(1, "early end".into());
        assert!(exam.validate_final().is_err());
    }

    #[test]
    fn challenge_requires_external_real_validation() {
        let mut exam = exam();
        exam.need_face = true;
        assert!(exam.check_challenge().is_err());
        exam.set_challenge_data(
            None,
            Some("real-token".into()),
            Some(json!({"validated":true})),
        );
        assert!(exam.check_challenge().is_ok());
        exam.need_captcha = true;
        assert!(exam.check_challenge().is_err());
        exam.set_challenge_data(Some("real-validate".into()), None, None);
        assert!(exam.check_challenge().is_ok());
    }

    #[test]
    fn dynamic_receipt_is_validated_atomically_before_confirming_answer() {
        let mut exam = exam();
        exam.enc = "old".into();
        exam.last_update_time = 10;
        assert!(
            exam.accept_submission(&json!({"status":"success","data":"20|bad|new"}), 1)
                .is_err()
        );
        assert_eq!(exam.enc, "old");
        assert_eq!(exam.last_update_time, 10);
        assert!(exam.confirmed.is_empty());
        exam.accept_submission(&json!({"status":"success","data":"20|30|new"}), 1)
            .unwrap();
        assert_eq!(exam.enc, "new");
        assert_eq!(exam.last_update_time, 20);
        assert_eq!(exam.enc_remain_time, 30);
        assert!(exam.confirmed.contains(&1));
    }

    #[test]
    fn request_keeps_dynamic_parameters_and_false_distinct_from_unanswered() {
        let mut exam = exam();
        exam.started = true;
        exam.enc = "latest".into();
        exam.last_update_time = 20;
        exam.enc_remain_time = 30;
        exam.seen.insert(2, 9);
        let mut question = Question {
            id: 9,
            value: "判断".into(),
            kind: crate::model::QuestionType::TRUE_FALSE,
            options: Value::Null,
            answer: json!(false),
        };
        let (query, form) = exam.request(2, Some(&question), false).unwrap();
        assert!(query.contains(&("qid".into(), "9".into())));
        assert!(query.contains(&("tempSave".into(), "true".into())));
        assert_eq!(form["answer9"], "false");
        assert_eq!(form["enc"], "latest");
        assert_eq!(form["encLastUpdateTime"], "20");
        question.answer = Value::Null;
        assert!(exam.request(2, Some(&question), false).is_err());
    }
}
