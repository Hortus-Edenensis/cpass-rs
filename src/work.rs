use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use scraper::{Html, Selector};
use serde_json::{Value, json};

use crate::course::{Course, Task, scalar as string};
use crate::model::Question;
use crate::questions::{PageKind, ParsedPaper, parse_page, valid_answer, work_form};
use crate::transport::{HttpResponse, Params, Session};

const PAGE: &str = "https://mooc1-api.chaoxing.com/android/mworkspecial";
const COMMIT: &str = "https://mooc1-api.chaoxing.com/work/addStudentWorkNew";

pub struct Work {
    session: Session,
    course: Course,
    chapter_id: u64,
    uid: u64,
    work_id: String,
    school_id: String,
    job_id: String,
    ktoken: String,
    enc: String,
    paper: Option<ParsedPaper>,
}

fn required(form: &BTreeMap<String, String>, key: &str) -> Result<String> {
    form.get(key)
        .filter(|value| !value.is_empty())
        .cloned()
        .with_context(|| format!("作业表单缺少 {key}"))
}

fn confirm(value: Value) -> Result<Value> {
    ensure!(
        value.get("status") == Some(&Value::Bool(true)),
        "作业保存或交卷回执未确认成功"
    );
    Ok(value)
}

fn validate_final(paper: &ParsedPaper) -> Result<()> {
    let count = paper.declared_count.context("作业没有可信题目总数")?;
    ensure!(count > 0, "作业题目总数为零");
    ensure!(paper.parse_errors.is_empty(), "存在解析失败题目，不能交卷");
    ensure!(
        paper.questions.len() == count && paper.indices.len() == count,
        "作业题目未完整获取"
    );
    ensure!(
        paper.indices.iter().copied().collect::<BTreeSet<_>>() == (0..count).collect(),
        "作业原始题号未完整获取"
    );
    ensure!(
        paper
            .questions
            .iter()
            .map(|question| question.id)
            .collect::<BTreeSet<_>>()
            .len()
            == count,
        "作业题目 ID 重复"
    );
    ensure!(
        paper.questions.iter().all(valid_answer),
        "存在未完成题目，不能交卷"
    );
    Ok(())
}

impl Work {
    pub fn from_task(session: Session, task: &Task, uid: u64) -> Result<Self> {
        let work_id = string(&task.property["workid"]).context("作业任务缺少 workid")?;
        let resource = task.point().context("作业附件无法唯一定位")?;
        let job_id = string(&task.property["_jobid"])
            .or_else(|| string(&task.property["jobid"]))
            .or_else(|| string(&resource["jobid"]))
            .context("作业任务缺少 jobid")?;
        let ktoken = string(&task.attachment["defaults"]["ktoken"])
            .or_else(|| string(&resource["ktoken"]))
            .context("作业任务缺少 ktoken")?;
        let enc = string(&resource["enc"]).context("作业任务缺少 enc")?;
        Ok(Self {
            session,
            course: task.course.clone(),
            chapter_id: task.chapter_id,
            uid,
            work_id,
            school_id: string(&task.property["schoolid"]).unwrap_or_default(),
            job_id,
            ktoken,
            enc,
            paper: None,
        })
    }

    pub fn paper(&self) -> Option<&ParsedPaper> {
        self.paper.as_ref()
    }

    fn fetch_page(&self) -> Result<HttpResponse> {
        let work_id = if self.school_id.is_empty() {
            self.work_id.clone()
        } else {
            format!("{}-{}", self.school_id, self.work_id)
        };
        let params = vec![
            ("courseid".into(), self.course.id.to_string()),
            ("workid".into(), work_id),
            ("jobid".into(), self.job_id.clone()),
            ("needRedirect".into(), "true".into()),
            ("knowledgeid".into(), self.chapter_id.to_string()),
            ("userid".into(), self.uid.to_string()),
            ("ut".into(), "s".into()),
            ("clazzId".into(), self.course.class_id.to_string()),
            ("cpi".into(), self.course.cpi.to_string()),
            ("ktoken".into(), self.ktoken.clone()),
            ("enc".into(), self.enc.clone()),
        ];
        self.session.get(PAGE, &params)
    }

    pub fn fetch_resources(&self) -> Result<crate::resources::ResourceManifest> {
        let response = self.fetch_page()?;
        crate::resources::extract(&response.text()?, &response.url)
    }

    pub fn fetch_review(&self) -> Result<crate::resources::ReviewedWork> {
        let response = self.fetch_page()?;
        crate::resources::parse_review(&response.text()?, &response.url)
    }

    pub fn fetch(&mut self) -> Result<ParsedPaper> {
        let text = self.fetch_page()?.text()?;
        let html = Html::parse_document(&text);
        if html
            .select(&Selector::parse("p.blankTips").unwrap())
            .next()
            .is_some()
        {
            bail!("作业页面拒绝访问或作业不可用");
        }
        if html
            .select(&Selector::parse("title").unwrap())
            .any(|node| node.text().collect::<String>().contains("已批阅"))
        {
            bail!("作业已批阅，不支持重新提交");
        }
        ensure!(
            html.select(&Selector::parse("form#form1").unwrap())
                .next()
                .is_some(),
            "作业表单尚未创建"
        );
        let paper = parse_page(&text, PageKind::Work)?;
        for key in ["workAnswerId", "workRelationId", "fullScore", "enc_work"] {
            required(&paper.form, key)?;
        }
        self.paper = Some(paper.clone());
        Ok(paper)
    }

    pub fn submit(&mut self, index: usize, question: &Question) -> Result<Value> {
        ensure!(valid_answer(question), "答案无效或未完成");
        let paper = self.paper.as_mut().context("请先获取作业题目")?;
        let position = paper
            .indices
            .iter()
            .position(|value| *value == index)
            .context("题目原始索引不存在")?;
        ensure!(
            paper.questions[position].id == question.id,
            "题目 ID 与缓存不一致"
        );
        paper.questions[position] = question.clone();
        Ok(json!({"status": true, "scope": "cache", "index": index,
            "question": question.value, "answer": question.answer}))
    }

    fn request(&self, save: bool) -> Result<(Params, BTreeMap<String, String>)> {
        let paper = self.paper.as_ref().context("请先获取作业题目")?;
        if !save {
            validate_final(paper)?;
        }
        let answer_id = required(&paper.form, "workAnswerId")?;
        let relation_id = required(&paper.form, "workRelationId")?;
        let enc_work = required(&paper.form, "enc_work")?;
        let mut query = vec![
            ("_classId".into(), self.course.class_id.to_string()),
            ("courseid".into(), self.course.id.to_string()),
            ("token".into(), enc_work.clone()),
            ("workAnswerId".into(), answer_id.clone()),
            ("ua".into(), "app".into()),
        ];
        if save {
            query.extend([
                ("formType2".into(), "post".into()),
                ("saveStatus".into(), "1".into()),
                ("version".into(), "1".into()),
                ("tempsave".into(), "1".into()),
            ]);
        } else {
            query.extend([
                ("keyboardDisplayRequiresUserAction".into(), "1".into()),
                ("workid".into(), relation_id.clone()),
                ("cpi".into(), self.course.cpi.to_string()),
                ("jobid".into(), self.job_id.clone()),
                ("knowledgeid".into(), self.chapter_id.to_string()),
            ]);
        }
        let mut form: Params = vec![
            ("pyFlag".into(), if save { "1" } else { "" }.into()),
            ("courseId".into(), self.course.id.to_string()),
            ("classId".into(), self.course.class_id.to_string()),
            ("api".into(), "1".into()),
            ("mooc".into(), "0".into()),
            ("workAnswerId".into(), answer_id),
            (
                "totalQuestionNum".into(),
                required(&paper.form, "totalQuestionNum")?,
            ),
            ("fullScore".into(), required(&paper.form, "fullScore")?),
            ("knowledgeid".into(), self.chapter_id.to_string()),
            ("oldSchoolId".into(), "".into()),
            ("oldWorkId".into(), self.work_id.clone()),
            ("jobid".into(), self.job_id.clone()),
            ("workRelationId".into(), relation_id),
            ("enc_work".into(), enc_work),
            ("isphone".into(), "true".into()),
            ("userId".into(), self.uid.to_string()),
            ("workTimesEnc".into(), "".into()),
        ];
        form.extend(work_form(&paper.questions));
        Ok((query, form.into_iter().collect()))
    }

    pub fn save(&self) -> Result<Value> {
        let (query, form) = self.request(true)?;
        confirm(self.session.post_form(COMMIT, &query, &form)?.json()?)
    }

    pub fn final_submit(&self) -> Result<Value> {
        let (query, form) = self.request(false)?;
        confirm(self.session.post_form(COMMIT, &query, &form)?.json()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::QuestionType;

    fn paper(answer: Value) -> ParsedPaper {
        ParsedPaper {
            questions: vec![Question {
                id: 9,
                value: "判断".into(),
                kind: QuestionType::TRUE_FALSE,
                options: Value::Null,
                answer,
            }],
            indices: vec![0],
            parse_errors: BTreeMap::new(),
            form: BTreeMap::new(),
            declared_count: Some(1),
            title: "作业".into(),
        }
    }

    #[test]
    fn final_guard_keeps_false_and_rejects_missing_or_parse_errors() {
        let mut paper = paper(json!(false));
        assert!(validate_final(&paper).is_ok());
        paper.declared_count = Some(2);
        assert!(validate_final(&paper).is_err());
        paper.declared_count = Some(1);
        paper.parse_errors.insert(1, "missing".into());
        assert!(validate_final(&paper).is_err());
        assert!(validate_final(&self::paper(Value::Null)).is_err());
    }

    #[test]
    fn only_boolean_remote_success_is_accepted() {
        assert!(confirm(json!({"status":true})).is_ok());
        for value in [
            json!({"status":false}),
            json!({"status":"success"}),
            json!({}),
            Value::Null,
        ] {
            assert!(confirm(value).is_err());
        }
    }

    fn work() -> Work {
        let session = Session::new(5, 0, Some("http://127.0.0.1:1")).unwrap();
        let course = Course {
            id: 1,
            class_id: 2,
            cpi: 3,
            key: 4,
            name: "course".into(),
            teacher: "teacher".into(),
        };
        let task = Task {
            kind: crate::course::TaskKind::Work,
            card_index: 0,
            chapter_id: 5,
            course,
            property: json!({"workid":"6","_jobid":"job"}),
            attachment: json!({"defaults":{"ktoken":"k"},
                "attachments":[{"jobid":"job","enc":"e","property":{"workid":"6"}}]}),
        };
        let mut work = Work::from_task(session, &task, 7).unwrap();
        let mut paper = paper(json!(false));
        paper.form = [
            ("workAnswerId", "8"),
            ("workRelationId", "9"),
            ("enc_work", "enc"),
            ("totalQuestionNum", "1"),
            ("fullScore", "5"),
            ("answer100", "stale-hidden-answer"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
        work.paper = Some(paper);
        work
    }

    #[test]
    fn forms_keep_false_and_use_explicit_save_fields_without_hidden_answers() {
        let mut work = work();
        let (query, form) = work.request(false).unwrap();
        assert!(query.contains(&("cpi".into(), "3".into())));
        assert!(!query.iter().any(|(key, _)| key == "cpi:"));
        assert_eq!(form["answer9"], "false");
        assert_eq!(form["answerwqbid"], "9");
        assert!(!form.contains_key("answer100"));
        work.paper.as_mut().unwrap().questions[0].answer = Value::Null;
        assert!(work.request(false).is_err());
        let (query, form) = work.request(true).unwrap();
        assert!(query.contains(&("tempsave".into(), "1".into())));
        assert_eq!(form["pyFlag"], "1");
        assert!(!form.contains_key("answer9"));
    }

    #[test]
    fn local_receipt_requires_matching_original_index_and_id() {
        let mut work = work();
        let mut question = work.paper().unwrap().questions[0].clone();
        let receipt = work.submit(0, &question).unwrap();
        assert_eq!(receipt["scope"], "cache");
        assert_eq!(receipt["answer"], false);
        assert!(work.submit(1, &question).is_err());
        question.id = 10;
        assert!(work.submit(0, &question).is_err());
    }
}
