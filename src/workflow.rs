use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};

use crate::exam::Exam;
use crate::model::{Question, QuestionSet};
use crate::questions::{ParsedPaper, fill, valid_answer};
use crate::search::Searchers;
use crate::work::Work;

#[derive(Debug, Default, Serialize)]
pub struct SolveReport {
    pub existing: usize,
    pub matched: usize,
    pub cached: usize,
    pub submitted: usize,
    pub incomplete: usize,
    pub saved: bool,
    pub final_submitted: bool,
    pub parse_errors: BTreeMap<usize, String>,
    pub failures: Vec<Value>,
    pub warnings: Vec<String>,
}

impl SolveReport {
    fn failure(&mut self, index: usize, question: Option<&Question>, reason: impl ToString) {
        self.failures
            .push(json!({"index":index,"id":question.map(|q|q.id),"reason":reason.to_string()}));
    }
    pub fn complete(&self) -> bool {
        self.incomplete == 0 && self.parse_errors.is_empty() && self.failures.is_empty()
    }
}

pub fn resolve_question(question: &mut Question, searchers: &Searchers) -> Result<bool> {
    if valid_answer(question) {
        return Ok(true);
    }
    let candidates = searchers.search(question);
    ensure!(
        !candidates.iter().any(|c| c.conflict),
        "答案来源冲突，保持未完成"
    );
    let answers: Vec<_> = candidates
        .into_iter()
        .filter(|c| c.error.is_none())
        .map(|c| c.answer)
        .collect();
    fill(question, &answers)
}

pub fn resolve_paper(paper: &mut ParsedPaper, searchers: &Searchers) -> SolveReport {
    let mut report = SolveReport {
        parse_errors: paper.parse_errors.clone(),
        ..Default::default()
    };
    let mut ids = std::collections::BTreeSet::new();
    for (position, question) in paper.questions.iter_mut().enumerate() {
        let Some(&index) = paper.indices.get(position) else {
            report.incomplete += 1;
            report.failure(position, Some(question), "缺少原始题目索引");
            continue;
        };
        if question.id == 0 || !ids.insert(question.id) {
            report.incomplete += 1;
            report.failure(index, Some(question), "题目 ID 缺失或重复");
            continue;
        }
        if valid_answer(question) {
            report.existing += 1;
            continue;
        }
        match resolve_question(question, searchers) {
            Ok(true) => report.matched += 1,
            Ok(false) => {
                report.incomplete += 1;
                report.failure(index, Some(question), "未匹配／未完成");
            }
            Err(error) => {
                report.incomplete += 1;
                report.failure(index, Some(question), error);
            }
        }
    }
    report.incomplete += paper.parse_errors.len();
    if paper.questions.is_empty() {
        report.incomplete += 1;
        report.failure(0, None, "没有可验证题目");
    }
    if let Some(count) = paper.declared_count
        && count != paper.questions.len() + paper.parse_errors.len()
    {
        report.incomplete += count
            .saturating_sub(paper.questions.len() + paper.parse_errors.len())
            .max(1);
        report.failure(0, None, "题目总数与解析结果不一致");
    }
    report
}

pub fn exported(paper: &ParsedPaper, id: Value, export_type: u8) -> QuestionSet {
    QuestionSet {
        id,
        title: paper.title.clone(),
        export_type,
        questions: paper.questions.clone(),
    }
}

pub fn run_work(
    work: &mut Work,
    searchers: &Searchers,
    commit: bool,
    final_submit: bool,
    fallback_save: bool,
) -> Result<(ParsedPaper, SolveReport)> {
    ensure!(!final_submit || commit, "交卷需要同时指定 --commit");
    let mut paper = work.fetch()?;
    let existing: Vec<_> = paper.questions.iter().map(valid_answer).collect();
    let mut report = resolve_paper(&mut paper, searchers);
    for (position, question) in paper.questions.iter().enumerate() {
        if existing[position] || !valid_answer(question) {
            continue;
        }
        match work.submit(paper.indices[position], question) {
            Ok(receipt) if receipt["status"] == true && receipt["scope"] == "cache" => {
                report.cached += 1
            }
            Ok(_) => {
                report.incomplete += 1;
                report.failure(paper.indices[position], Some(question), "单题缓存回执无效");
            }
            Err(error) => {
                report.incomplete += 1;
                report.failure(paper.indices[position], Some(question), error);
            }
        }
    }
    if commit && final_submit && report.complete() {
        match work.final_submit() {
            Ok(_) => report.final_submitted = true,
            Err(error) => report.failure(0, None, error),
        }
    } else if commit && (report.complete() || fallback_save) {
        match work.save() {
            Ok(_) => report.saved = true,
            Err(error) => report.failure(0, None, error),
        }
    }
    Ok((paper, report))
}

pub fn run_exam(
    exam: &mut Exam,
    searchers: &Searchers,
    commit: bool,
    final_submit: bool,
    delay: f64,
) -> Result<(ParsedPaper, SolveReport)> {
    ensure!(!final_submit || commit, "交卷需要同时指定 --commit");
    ensure!(
        delay.is_finite() && (0.0..=3600.0).contains(&delay),
        "提交延迟无效"
    );
    let mut paper = exam.preview()?;
    if !commit {
        let report = resolve_paper(&mut paper, searchers);
        return Ok((paper, report));
    }
    let mut report = SolveReport {
        parse_errors: paper.parse_errors.clone(),
        ..Default::default()
    };
    for position in 0..paper.questions.len() {
        let index = paper.indices[position];
        let expected_id = paper.questions[position].id;
        let fetched = match exam.fetch(index) {
            Ok(Some(page)) => page,
            Ok(None) => {
                report.incomplete += 1;
                report.failure(index, None, "考试提前结束，预览题目未读取");
                continue;
            }
            Err(error) => {
                report.incomplete += 1;
                report.failure(index, None, error);
                continue;
            }
        };
        report.parse_errors.extend(fetched.parse_errors.clone());
        let Some(mut question) = fetched.questions.into_iter().find(|q| q.id == expected_id) else {
            report.incomplete += 1;
            report.failure(index, None, "当前题目与预览 ID 不一致");
            continue;
        };
        if valid_answer(&question) {
            report.existing += 1;
            paper.questions[position] = question;
            continue;
        }
        match resolve_question(&mut question, searchers) {
            Ok(true) => report.matched += 1,
            Ok(false) => {
                report.incomplete += 1;
                report.failure(index, Some(&question), "未匹配／未完成");
                continue;
            }
            Err(error) => {
                report.incomplete += 1;
                report.failure(index, Some(&question), error);
                continue;
            }
        }
        if delay > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(delay));
        }
        match exam.submit(index, &question) {
            Ok(_) => report.submitted += 1,
            Err(error) => {
                report.incomplete += 1;
                report.failure(index, Some(&question), error);
            }
        }
        paper.questions[position] = question;
    }
    report.parse_errors.extend(exam.parse_errors().clone());
    report.incomplete += report.parse_errors.len();
    if paper.questions.is_empty() || exam.expected_count() != Some(paper.questions.len()) {
        report.incomplete += 1;
        report.failure(0, None, "考试没有完整且可信的题目边界");
    }
    if final_submit && report.complete() {
        match exam.final_submit() {
            Ok(_) => report.final_submitted = true,
            Err(error) => report.failure(0, None, error),
        }
    }
    Ok((paper, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::QuestionType;
    #[test]
    fn existing_false_skips_search_and_empty_papers_are_incomplete() {
        let searchers = Searchers::new(&[]).unwrap();
        let mut paper = ParsedPaper {
            questions: vec![Question {
                id: 1,
                value: "判断".into(),
                kind: QuestionType::TRUE_FALSE,
                options: Value::Null,
                answer: json!(false),
            }],
            indices: vec![0],
            parse_errors: BTreeMap::new(),
            form: BTreeMap::new(),
            declared_count: Some(1),
            title: String::new(),
        };
        let report = resolve_paper(&mut paper, &searchers);
        assert_eq!(report.existing, 1);
        assert!(report.complete());
        paper.questions.clear();
        paper.indices.clear();
        assert!(!resolve_paper(&mut paper, &searchers).complete());
    }
}
