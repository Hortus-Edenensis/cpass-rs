use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde_json::{Map, Value};

use crate::model::{Question, QuestionType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    Work,
    Exam,
}

#[derive(Debug, Clone)]
pub struct ParsedPaper {
    pub questions: Vec<Question>,
    pub indices: Vec<usize>,
    pub parse_errors: BTreeMap<usize, String>,
    pub form: BTreeMap<String, String>,
    pub declared_count: Option<usize>,
    pub title: String,
}

pub fn normalize_text(text: &str) -> String {
    html_escape::decode_html_entities(text)
        .chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}'
            )
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn rx(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex")
}
fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("internal CSS selector")
}
fn first<'a>(node: ElementRef<'a>, css: &str) -> Option<ElementRef<'a>> {
    node.select(&selector(css)).next()
}
fn render(
    node: ElementRef<'_>,
    skip_heading: bool,
    skip_label: Option<ElementRef<'_>>,
    out: &mut String,
) {
    if Some(node) == skip_label {
        return;
    }
    let name = node.value().name();
    if matches!(name, "script" | "style" | "input" | "button") || (skip_heading && name == "h3") {
        return;
    }
    if name == "br" {
        out.push('\n');
        return;
    }
    let block = matches!(name, "p" | "div" | "li");
    if block {
        out.push('\n');
    }
    for child in node.children() {
        if let Some(el) = ElementRef::wrap(child) {
            render(el, skip_heading, skip_label, out);
        } else if let Some(text) = child.value().as_text() {
            if text.trim().is_empty() && text.contains('\n') {
                continue;
            }
            out.push_str(&rx(r"\s+").replace_all(text, " "));
        }
    }
    if block {
        out.push('\n');
    }
}
fn text(node: ElementRef<'_>) -> String {
    render_text(node, false, None)
}
fn render_text(
    node: ElementRef<'_>,
    skip_heading: bool,
    skip_label: Option<ElementRef<'_>>,
) -> String {
    let mut out = String::new();
    render(node, skip_heading, skip_label, &mut out);
    out.lines()
        .map(normalize_text)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
const TYPE_NAMES: &str = "单选题|多选题|填空题|判断题|简答题|名词解释|论述题|计算题|其它|分录题|资料题|连线题|排序题|完型填空|阅读理解|口语题|听力题|共用选项题|测评题";
fn type_label(node: ElementRef<'_>) -> Option<ElementRef<'_>> {
    let whole = node.text().collect::<String>();
    node.select(&selector("span, strong, b")).find(|tag| {
        let raw = tag.text().collect::<String>();
        rx(&format!("^(?:{TYPE_NAMES})$")).is_match(&normalize_text(&raw))
            && whole.split_once(&raw).is_some_and(|(prefix, _)| {
                rx(r"^(?:\d+\s*[.．、])?$").is_match(&normalize_text(prefix))
            })
    })
}
fn question_type(raw: Option<&str>, title: &str) -> QuestionType {
    if let Some(value) = raw.and_then(|v| v.trim().parse::<i64>().ok()) {
        return QuestionType(value);
    }
    let title = normalize_text(title);
    for id in [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 14, 15, 18, 19, 20, 21,
    ] {
        let kind = QuestionType(id);
        if rx(&format!(
            r"^(?:\d+\s*[.．、]\s*)?[（(【\[]?\s*{}(?:$|[\s（(）)【\[】\],，:：])",
            kind.name()
        ))
        .is_match(&title)
        {
            return kind;
        }
    }
    QuestionType(8)
}
fn body(node: ElementRef<'_>, exam: bool) -> Result<String> {
    let text = render_text(node, exam, type_label(node));
    // Only leading display metadata is removed; digits and parentheses in the body remain literal.
    let number = rx(r"^\s*\d+\s*[.．、]");
    let text = if let Some(prefix) = number.find(&text) {
        let remaining = &text[prefix.end()..];
        if remaining.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            text.as_str()
        } else {
            remaining.trim_start()
        }
    } else {
        text.as_str()
    };
    let text = rx(&format!(
        r"^\s*[（(【\[]\s*(?:{TYPE_NAMES})(?:\s*[,，]\s*\d+(?:\.\d+)?\s*分)?\s*[）)】\]]\s*"
    ))
    .replace(text, "")
    .into_owned();
    let text = rx(r"^\s*[（(]\s*\d+(?:\.\d+)?\s*分\s*[）)]\s*")
        .replace(&text, "")
        .trim()
        .to_string();
    if text.is_empty() {
        bail!("题干为空");
    }
    Ok(text)
}
fn positive_id(raw: Option<&str>) -> Result<u64> {
    let raw = raw.context("缺少可信题目 ID")?;
    if raw.is_empty() || !raw.bytes().all(|c| c.is_ascii_digit()) {
        bail!("缺少可信题目 ID");
    }
    let id = raw.parse::<u64>().context("题目 ID 无效")?;
    if id == 0 {
        bail!("缺少可信题目 ID");
    }
    Ok(id)
}
fn parse_question(node: ElementRef<'_>, page_kind: PageKind) -> Result<Question> {
    let id_input = first(node, "input[name='questionId']");
    let type_nodes: Vec<_> = node
        .select(&selector("input[id]"))
        .filter(|el| {
            el.value()
                .attr("id")
                .is_some_and(|id| rx(r"^answertype[0-9]+$").is_match(id))
        })
        .collect();
    if page_kind == PageKind::Work && type_nodes.len() > 1 {
        bail!("题目 ID 不唯一");
    }
    let raw_id = id_input
        .and_then(|el| el.value().attr("value"))
        .or_else(|| {
            if page_kind == PageKind::Work {
                type_nodes
                    .first()
                    .and_then(|el| el.value().attr("id"))
                    .map(|id| &id[10..])
            } else {
                None
            }
        })
        .or_else(|| node.value().attr("data"));
    let id = positive_id(raw_id)?;
    let (title_css, type_css, answer_css) = match page_kind {
        PageKind::Work => (
            "div.Py-m1-title",
            format!("input[id='answertype{id}']"),
            "input.answerInput".to_string(),
        ),
        PageKind::Exam => (
            "div.tit",
            format!("input[name='type{id}'], input[id='type{id}']"),
            format!(
                "input[id='answer{id}'], input[id='answers{id}'], input[name='answer{id}'], input[name='answers{id}']"
            ),
        ),
    };
    let title = first(node, title_css).context("缺少题干")?;
    let heading = if page_kind == PageKind::Exam {
        first(title, "h3")
    } else {
        None
    };
    let kind = question_type(
        first(node, &type_css).and_then(|el| el.value().attr("value")),
        &text(heading.or_else(|| type_label(title)).unwrap_or(title)),
    );
    let saved = first(node, &answer_css)
        .and_then(|el| el.value().attr("value"))
        .unwrap_or("");
    let mut q = Question {
        id,
        value: body(title, page_kind == PageKind::Exam)?,
        kind,
        options: Value::Null,
        answer: Value::Null,
    };
    match kind {
        QuestionType::SINGLE | QuestionType::MULTIPLE => {
            let option_css = if page_kind == PageKind::Work {
                "li.more-choose-item"
            } else {
                "div.answerList.radioList"
            };
            let mut options = Map::new();
            for option in node.select(&selector(option_css)) {
                let key = if page_kind == PageKind::Work {
                    first(option, "em.choose-opt").and_then(|el| el.value().attr("id-param"))
                } else {
                    option.value().attr("name")
                }
                .context("选项标号缺失")?;
                if key.is_empty() || options.contains_key(key) {
                    bail!("选项标号缺失或重复");
                }
                let content = if page_kind == PageKind::Work {
                    first(option, "div.choose-desc").context("缺少选项正文")?
                } else {
                    first(option, ".choose-desc, .option-content").unwrap_or(option)
                };
                let value = option_text(key, &text(first(content, "cc").unwrap_or(content)));
                if value.is_empty() {
                    bail!("选项正文为空");
                }
                options.insert(key.to_string(), Value::String(value));
            }
            if options.is_empty() {
                bail!("缺少选项");
            }
            q.options = Value::Object(options);
            if !saved.is_empty() {
                q.answer = Value::String(saved.to_string());
            }
        }
        QuestionType::BLANK => {
            let (css, label_css, field_css) = if page_kind == PageKind::Work {
                ("ul.blankList2 > li", "span", "input.blankInp2")
            } else {
                (
                    "div.completionList.objectAuswerList",
                    "span.grayTit",
                    "textarea.blanktextarea",
                )
            };
            let mut options = Vec::new();
            let mut answers = Vec::new();
            for blank in node.select(&selector(css)) {
                let field = first(blank, field_css).context("缺少填空输入框")?;
                options.push(Value::String(
                    first(blank, label_css).map(text).unwrap_or_default(),
                ));
                answers.push(Value::String(if page_kind == PageKind::Work {
                    field.value().attr("value").unwrap_or("").to_string()
                } else {
                    field.text().collect()
                }));
            }
            if options.is_empty() {
                bail!("缺少填空项");
            }
            q.options = Value::Array(options);
            q.answer = Value::Array(answers);
        }
        QuestionType::TRUE_FALSE => {
            q.answer = match saved.trim().to_ascii_lowercase().as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => Value::Null,
            };
        }
        _ => {}
    }
    Ok(q)
}

pub fn parse_page(source: &str, kind: PageKind) -> Result<ParsedPaper> {
    let html = Html::parse_document(source);
    let root = html.root_element();
    if let Some(error) = first(root, "p.blankTips") {
        bail!("{}", text(error));
    }
    let mut form = BTreeMap::new();
    for input in html.select(&selector("input")) {
        if input
            .value()
            .attr("type")
            .is_some_and(|v| !v.eq_ignore_ascii_case("hidden"))
        {
            continue;
        }
        if let Some(key) = input
            .value()
            .attr("name")
            .or_else(|| input.value().attr("id"))
        {
            form.insert(
                key.to_string(),
                input.value().attr("value").unwrap_or("").to_string(),
            );
        }
    }
    let declared_count = form
        .get("totalQuestionNum")
        .filter(|v| !v.is_empty() && v.bytes().all(|c| c.is_ascii_digit()))
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0);
    let css = if kind == PageKind::Work {
        "div.Py-mian1"
    } else {
        "div.questionWrap.singleQuesId.ans-cc-exam"
    };
    let nodes: Vec<_> = html.select(&selector(css)).collect();
    if nodes.is_empty() && (kind == PageKind::Exam || declared_count.is_none()) {
        bail!("页面缺少题目节点或可信题目总数");
    }
    let title = first(root, "h3.py-Title, h3.chapter-title, span.overHidden2")
        .or_else(|| first(root, "title"))
        .map(text)
        .unwrap_or_default();
    let mut paper = ParsedPaper {
        questions: Vec::new(),
        indices: Vec::new(),
        parse_errors: BTreeMap::new(),
        form,
        declared_count,
        title,
    };
    if kind == PageKind::Work {
        match declared_count {
            Some(count) => {
                for index in nodes.len()..count {
                    paper.parse_errors.insert(index, "缺少题目节点".to_string());
                }
                for index in count..nodes.len() {
                    paper
                        .parse_errors
                        .insert(index, "题目节点超出声明总数".to_string());
                }
            }
            None => {
                paper.parse_errors.insert(0, "题目总数无效".to_string());
            }
        }
    }
    let mut ids = BTreeSet::new();
    for (index, node) in nodes.into_iter().enumerate() {
        match parse_question(node, kind) {
            Ok(question) if ids.insert(question.id) => {
                paper.indices.push(index);
                paper.questions.push(question);
            }
            Ok(question) => {
                paper
                    .parse_errors
                    .insert(index, format!("题目 ID 重复: {}", question.id));
            }
            Err(err) => {
                paper.parse_errors.insert(index, err.to_string());
            }
        }
    }
    Ok(paper)
}

pub fn valid_answer(q: &Question) -> bool {
    match q.kind {
        QuestionType::SINGLE => q
            .answer
            .as_str()
            .zip(q.options.as_object())
            .is_some_and(|(a, o)| o.contains_key(a)),
        QuestionType::MULTIPLE => {
            q.answer
                .as_str()
                .zip(q.options.as_object())
                .is_some_and(|(a, o)| {
                    !a.is_empty()
                        && a.chars().collect::<BTreeSet<_>>().len() == a.chars().count()
                        && a.chars().all(|c| o.contains_key(&c.to_string()))
                })
        }
        QuestionType::TRUE_FALSE => q.answer.is_boolean(),
        QuestionType::BLANK => {
            q.answer
                .as_array()
                .zip(q.options.as_array())
                .is_some_and(|(a, o)| {
                    !o.is_empty()
                        && a.len() == o.len()
                        && a.iter()
                            .all(|v| v.as_str().is_some_and(|s| !normalize_text(s).is_empty()))
                })
        }
        _ => false,
    }
}

fn label(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'Ａ'..='Ｚ' | 'ａ'..='ｚ' => char::from_u32(c as u32 - 0xfee0)
                .unwrap()
                .to_ascii_uppercase(),
            _ => c.to_ascii_uppercase(),
        })
        .collect()
}
fn option_text(key: &str, value: &str) -> String {
    let text = normalize_text(value);
    if let Some(m) = rx(r"^([A-Za-zＡ-Ｚａ-ｚ])[.．、:：)）]\s*").captures(&text)
        && label(&m[1]) == label(key)
    {
        return text[m.get(0).unwrap().end()..].to_string();
    }
    text
}
fn choice(q: &Question, raw: &str) -> Option<String> {
    let text = normalize_text(raw);
    let options = q.options.as_object()?;
    if text.is_empty() {
        return None;
    }
    if let Some(m) = rx(r"^([A-Za-zＡ-Ｚａ-ｚ])[.．、:：)）]\s*(.*)$").captures(&text) {
        let keys: Vec<_> = options
            .keys()
            .filter(|k| label(k) == label(&m[1]))
            .collect();
        if keys.len() != 1 {
            return None;
        }
        let key = keys[0];
        let body = normalize_text(&m[2]);
        return (body.is_empty() || body == option_text(key, options[key].as_str()?))
            .then(|| key.clone());
    }
    let keys: Vec<_> = options
        .iter()
        .filter(|(k, v)| {
            label(k) == label(&text) || v.as_str().is_some_and(|v| option_text(k, v) == text)
        })
        .map(|(k, _)| k.clone())
        .collect();
    (keys.len() == 1).then(|| keys[0].clone())
}
fn has_analysis(text: &str) -> bool {
    rx(r"(?i)(?:^|[,，;；。.\s])(?:解析|解释|理由|说明|analysis|explanation|reason)\s*[:：]")
        .is_match(&normalize_text(text))
}
fn answer_field(value: &Value) -> Option<Value> {
    let mut answer = value.clone();
    if let Value::String(raw) = &answer {
        let decoded = html_escape::decode_html_entities(raw);
        let mut text = decoded.trim().to_string();
        if text.starts_with("```") && text.ends_with("```") {
            let lines: Vec<_> = text.lines().collect();
            if lines.len() < 3 {
                return None;
            }
            text = lines[1..lines.len() - 1].join("\n").trim().to_string();
        }
        answer = if text.starts_with('{') || text.starts_with('[') {
            serde_json::from_str(&text).ok()?
        } else {
            Value::String(text)
        };
    }
    if let Value::Object(fields) = &answer {
        let values: Vec<_> = ["answer", "答案"]
            .iter()
            .filter_map(|key| fields.get(*key))
            .collect();
        if values.len() != 1 {
            return None;
        }
        answer = values[0].clone();
    }
    if let Value::String(raw) = &answer {
        let split = rx(r"(?i)\n\s*(?:解析|解释|理由|说明|analysis|explanation|reason)\s*[:：]");
        let text = split.split(raw).next()?;
        let text = rx(r"(?i)^\s*(?:参考答案|正确答案|答案|answer)\s*[:：]\s*")
            .replace(text, "")
            .to_string();
        if has_analysis(&text) {
            return None;
        }
        answer = Value::String(text);
    }
    Some(answer)
}

pub fn normalize_answer(q: &Question, raw: &Value) -> Option<Value> {
    if !q.kind.supported() {
        return None;
    }
    if matches!(q.kind, QuestionType::SINGLE | QuestionType::MULTIPLE)
        && let Some(text) = raw.as_str().filter(|s| s.trim().starts_with(['[', '{']))
        && let Some(literal) = choice(q, text)
    {
        let decoded = answer_field(raw).and_then(|value| normalize_decoded(q, &value));
        return (decoded.is_none() || decoded == Some(Value::String(literal.clone())))
            .then_some(Value::String(literal));
    }
    normalize_decoded(q, &answer_field(raw)?)
}
fn normalize_decoded(q: &Question, answer: &Value) -> Option<Value> {
    if q.kind == QuestionType::TRUE_FALSE {
        if let Some(v) = answer.as_bool() {
            return Some(Value::Bool(v));
        }
        if let Some(v @ (0 | 1)) = answer.as_i64() {
            return Some(Value::Bool(v == 1));
        }
        let text = normalize_text(answer.as_str()?)
            .trim_matches(['。', '.'])
            .to_ascii_lowercase();
        return match text.as_str() {
            "对" | "正确" | "是" | "√" | "true" | "yes" | "1" => Some(Value::Bool(true)),
            "错" | "错误" | "否" | "不正确" | "不对" | "不是" | "×" | "✗" | "false" | "no"
            | "0" => Some(Value::Bool(false)),
            _ => None,
        };
    }
    if q.kind == QuestionType::BLANK {
        let options = q.options.as_array()?;
        let values = if let Some(s) = answer.as_str() {
            s.split('#').map(|s| Value::String(s.to_string())).collect()
        } else {
            answer.as_array()?.clone()
        };
        if options.is_empty() || values.len() != options.len() {
            return None;
        }
        let mut blanks = Vec::new();
        for value in &values {
            let value = value.as_str()?;
            if has_analysis(value) || normalize_text(value).is_empty() {
                return None;
            }
            blanks.push(Value::String(normalize_text(value)));
        }
        if let Some(previous) = q.answer.as_array() {
            if previous.len() != blanks.len() {
                return None;
            }
            for (index, old) in previous.iter().enumerate() {
                if let Some(old_text) = old.as_str().filter(|s| !normalize_text(s).is_empty()) {
                    if normalize_text(old_text) != blanks[index].as_str()? {
                        return None;
                    }
                    blanks[index] = old.clone();
                }
            }
        }
        return Some(Value::Array(blanks));
    }
    if q.kind == QuestionType::SINGLE {
        let text = if let Some(s) = answer.as_str() {
            s
        } else {
            let parts = answer.as_array()?;
            if parts.len() != 1 {
                return None;
            }
            parts[0].as_str()?
        };
        return choice(q, text).map(Value::String);
    }
    let mut parts = Vec::new();
    let mut whole = None;
    if let Some(text) = answer.as_str() {
        whole = choice(q, text);
        parts = rx(r"[#;；,，、|\n]+")
            .split(text)
            .map(str::to_string)
            .collect();
        let compact = label(&text.split_whitespace().collect::<String>());
        if parts.len() == 1
            && !compact.is_empty()
            && compact.bytes().all(|c| c.is_ascii_uppercase())
        {
            parts = compact.chars().map(|c| c.to_string()).collect();
        }
    } else {
        for value in answer.as_array()? {
            parts.push(value.as_str()?.to_string());
        }
    }
    if parts.is_empty() {
        return None;
    }
    let keys: Option<BTreeSet<_>> = parts.iter().map(|p| choice(q, p)).collect();
    if let Some(whole) = whole {
        if keys
            .as_ref()
            .is_some_and(|keys| keys.len() != 1 || !keys.contains(&whole))
        {
            return None;
        }
        return Some(Value::String(whole));
    }
    let keys = keys?;
    let canonical = q
        .options
        .as_object()?
        .keys()
        .filter(|key| keys.contains(*key))
        .cloned()
        .collect::<String>();
    (!canonical.is_empty()).then_some(Value::String(canonical))
}

pub fn fill(q: &mut Question, candidates: &[Value]) -> Result<bool> {
    if valid_answer(q) {
        return Ok(true);
    }
    let mut found = None;
    for candidate in candidates {
        if let Some(value) = normalize_answer(q, candidate) {
            if found.as_ref().is_some_and(|prior| *prior != value) {
                bail!("答案来源冲突");
            }
            found = Some(value);
        }
    }
    if let Some(value) = found {
        q.answer = value;
        return Ok(true);
    }
    Ok(false)
}

pub fn work_form(questions: &[Question]) -> BTreeMap<String, String> {
    let questions: Vec<_> = questions.iter().filter(|q| valid_answer(q)).collect();
    let mut form = BTreeMap::from([(
        "answerwqbid".to_string(),
        questions
            .iter()
            .map(|q| q.id.to_string())
            .collect::<Vec<_>>()
            .join(","),
    )]);
    for q in questions {
        form.insert(format!("answertype{}", q.id), q.kind.0.to_string());
        if q.kind == QuestionType::BLANK {
            let answers = q.answer.as_array().unwrap();
            form.insert(format!("tiankongsize{}", q.id), answers.len().to_string());
            for (index, value) in answers.iter().enumerate() {
                form.insert(
                    format!("answer{}{}", q.id, index + 1),
                    value.as_str().unwrap().to_string(),
                );
            }
        } else {
            form.insert(format!("answer{}", q.id), wire_answer(q));
        }
    }
    form
}
fn wire_answer(q: &Question) -> String {
    q.answer
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| q.answer.as_bool().unwrap().to_string())
}
pub fn exam_form(q: &Question) -> Result<BTreeMap<String, String>> {
    if !valid_answer(q) {
        return Err(anyhow!("答案无效或未完成"));
    }
    let mut form = BTreeMap::from([
        (format!("type{}", q.id), q.kind.0.to_string()),
        ("questionId".to_string(), q.id.to_string()),
        (format!("typeName{}", q.id), q.kind.name().to_string()),
        ("hidetext".to_string(), String::new()),
    ]);
    match q.kind {
        QuestionType::BLANK => {
            let answers = q.answer.as_array().unwrap();
            let mut numbers = String::new();
            for (index, answer) in answers.iter().enumerate() {
                form.insert(
                    format!("answer{}{}", q.id, index + 1),
                    answer.as_str().unwrap().to_string(),
                );
                numbers.push_str(&format!("{},", index + 1));
            }
            form.insert(format!("blankNum{}", q.id), numbers);
        }
        QuestionType::MULTIPLE => {
            form.insert(format!("answers{}", q.id), wire_answer(q));
        }
        _ => {
            form.insert(format!("answer{}", q.id), wire_answer(q));
        }
    }
    Ok(form)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn question(kind: QuestionType) -> Question {
        Question {
            id: 42,
            value: "题目".into(),
            kind,
            options: match kind {
                QuestionType::SINGLE | QuestionType::MULTIPLE => {
                    json!({"B":"乙", "A":"甲", "C":"丙"})
                }
                QuestionType::BLANK => json!(["空1", "空2"]),
                _ => Value::Null,
            },
            answer: Value::Null,
        }
    }

    fn work_node(id: &str, kind: &str) -> String {
        format!(
            r#"<div class="Py-mian1"><input id="answertype{id}" value="{kind}">
            <div class="Py-m1-title">1.<span>（单选题，5.0分）</span>第一<span>行</span><p>第二行<br>第三行</p></div>
            <input class="answerInput" value="">
            <li class="more-choose-item"><em class="choose-opt" id-param="B">B.</em><div class="choose-desc">乙<b>选项</b></div></li>
            <li class="more-choose-item"><em class="choose-opt" id-param="A">A.</em><div class="choose-desc"><cc>甲选项</cc></div></li>
            <ul class="blankList2"><li><span>空1</span><input class="blankInp2" value="已有"></li><li><span>空2</span><input class="blankInp2" value=""></li></ul>
            </div>"#
        )
    }

    #[test]
    fn html_parsing_keeps_body_metadata_indices_and_counts_separate() {
        let good = work_node("42", "0");
        for html in [&good, &good.replace('\n', "")] {
            let page = parse_page(
                &format!("<input id='totalQuestionNum' value='1'>{html}"),
                PageKind::Work,
            )
            .unwrap();
            assert!(page.parse_errors.is_empty());
            assert_eq!(page.questions[0].value, "第一行\n第二行\n第三行");
            assert_eq!(page.questions[0].options["B"], "乙选项");
            assert_eq!(page.form["totalQuestionNum"], "1");
        }
        let missing_id = work_node("", "0");
        let unknown = work_node("43", "12");
        let page = parse_page(
            &format!("<input id='totalQuestionNum' value='5'>{missing_id}{good}{good}{unknown}"),
            PageKind::Work,
        )
        .unwrap();
        assert_eq!(page.indices, vec![1, 3]);
        assert_eq!(
            page.parse_errors.keys().copied().collect::<Vec<_>>(),
            vec![0, 2, 4]
        );
        assert_eq!(page.questions[1].kind, QuestionType(12));
        assert!(!valid_answer(&page.questions[1]));
        let no_count = parse_page(&good, PageKind::Work).unwrap();
        assert!(no_count.parse_errors.contains_key(&0));
        let extra = parse_page(
            &format!(
                "<input id='totalQuestionNum' value='1'>{good}{}",
                work_node("43", "3")
            ),
            PageKind::Work,
        )
        .unwrap();
        assert!(extra.parse_errors.contains_key(&1));
    }

    #[test]
    fn type_fields_win_and_only_nonnumeric_fields_use_explicit_headings() {
        for (raw, expected) in [
            ("0", 0),
            ("3", 3),
            ("4", 4),
            ("12", 12),
            ("999", 999),
            ("bad", 0),
        ] {
            let page = parse_page(
                &format!(
                    "<input id='totalQuestionNum' value='1'>{}",
                    work_node("42", raw)
                ),
                PageKind::Work,
            )
            .unwrap();
            assert_eq!(page.questions[0].kind, QuestionType(expected));
        }
        let bare = work_node("42", "bad").replace(
            "<span>（单选题，5.0分）</span>",
            "<span>单选题</span>（5.0分）",
        );
        let page = parse_page(
            &format!("<input id='totalQuestionNum' value='1'>{bare}"),
            PageKind::Work,
        )
        .unwrap();
        assert_eq!(page.questions[0].kind, QuestionType::SINGLE);
        assert_eq!(page.questions[0].value, "第一行\n第二行\n第三行");
        let exam = r#"<form id="submitTest"><input id="enc" value="token"><div class="allAnswerList questionWrap singleQuesId ans-cc-exam" data="42"><input name="questionId" value="42"><input name="typeName42" value="0"><input name="type42" value="3"><div class="tit"><h3>单选题（5.0分）</h3>1.题<span>目</span><p>后段</p></div><input id="answer42" value="false"></div></form>"#;
        let page = parse_page(exam, PageKind::Exam).unwrap();
        assert_eq!(page.questions[0].kind, QuestionType::TRUE_FALSE);
        assert_eq!(page.questions[0].value, "题目\n后段");
        assert_eq!(page.questions[0].answer, json!(false));
        assert!(valid_answer(&page.questions[0]));
        let duplicate = parse_page(&format!("{exam}{exam}"), PageKind::Exam).unwrap();
        assert_eq!(duplicate.questions.len(), 1);
        assert!(duplicate.parse_errors.contains_key(&1));
    }

    #[test]
    fn single_and_multiple_choices_require_complete_unique_agreement() {
        let mut q = question(QuestionType::SINGLE);
        for raw in [
            json!("A"),
            json!("ａ"),
            json!("A. 甲"),
            json!("甲"),
            json!(["甲"]),
        ] {
            assert_eq!(normalize_answer(&q, &raw), Some(json!("A")));
        }
        for raw in [
            json!("A. 乙"),
            json!("甲解释"),
            json!(["A", "B"]),
            Value::Null,
        ] {
            assert_eq!(normalize_answer(&q, &raw), None);
        }
        q.options = json!({"A":"甲","B":"甲"});
        assert_eq!(normalize_answer(&q, &json!("甲")), None);
        let q = question(QuestionType::MULTIPLE);
        for raw in [json!("ＡＢＡ"), json!("甲#乙"), json!(["A", "B", "A"])] {
            assert_eq!(normalize_answer(&q, &raw), Some(json!("BA")));
        }
        assert_eq!(normalize_answer(&q, &json!(["A", "未知"])), None);
        assert_eq!(normalize_answer(&q, &json!("A;D")), None);
    }

    #[test]
    fn booleans_blanks_and_forms_preserve_existing_values() {
        let mut q = question(QuestionType::TRUE_FALSE);
        for raw in [
            json!(false),
            json!("FALSE"),
            json!("不正确"),
            json!("答案：错误"),
        ] {
            assert_eq!(normalize_answer(&q, &raw), Some(json!(false)));
        }
        for raw in [json!("这不正确"), json!("正确或者错误"), Value::Null] {
            assert_eq!(normalize_answer(&q, &raw), None);
        }
        assert!(exam_form(&q).is_err());
        q.answer = json!(false);
        assert!(valid_answer(&q));
        assert_eq!(exam_form(&q).unwrap()["answer42"], "false");
        assert!(fill(&mut q, &[json!(true)]).unwrap());
        assert_eq!(q.answer, json!(false));
        q.answer = json!("false");
        assert!(!valid_answer(&q));
        assert!(!work_form(&[q]).contains_key("answer42"));
        let mut q = question(QuestionType::BLANK);
        q.answer = json!([" 已有 ", null]);
        assert_eq!(
            normalize_answer(&q, &json!(["已有", "新"])),
            Some(json!([" 已有 ", "新"]))
        );
        for raw in [json!(["冲突", "新"]), json!(["已有"]), json!(["已有", ""])] {
            assert_eq!(normalize_answer(&q, &raw), None);
        }
        assert!(!work_form(&[q.clone()]).contains_key("answer421"));
        assert!(fill(&mut q, &[json!(["已有", "新"])]).unwrap());
        let form = work_form(&[q]);
        assert_eq!(form["answer421"], " 已有 ");
        assert_eq!(form["tiankongsize42"], "2");
    }

    #[test]
    fn explicit_answers_math_and_source_conflicts_are_conservative() {
        let mut q = question(QuestionType::SINGLE);
        q.options = json!({"A":"[0,1]","B":"x−y","C":"ABC"});
        assert_eq!(normalize_answer(&q, &json!("[0,1]")), Some(json!("A")));
        assert_eq!(
            normalize_answer(&q, &json!("答案：B\n解析：计算得到")),
            Some(json!("B"))
        );
        assert_eq!(
            normalize_answer(&q, &json!({"answer":"B","analysis":"推导"})),
            Some(json!("B"))
        );
        assert_eq!(
            normalize_answer(&q, &json!("答案：B，解析：计算得到")),
            None
        );
        assert_eq!(normalize_answer(&q, &json!("abc")), None);
        assert_eq!(
            normalize_text("Ａ + x² − y &amp; z\u{200b}"),
            "Ａ + x² − y & z"
        );
        assert!(fill(&mut q, &[json!("B"), json!("x−y")]).unwrap());
        q.answer = Value::Null;
        assert!(fill(&mut q, &[json!("A"), json!("B")]).is_err());
        assert!(q.answer.is_null());
        let mut blank = question(QuestionType::BLANK);
        blank.options = json!(["空1"]);
        assert_eq!(
            normalize_answer(&blank, &json!("北京，解析：北京是首都")),
            None
        );
        assert_eq!(
            normalize_answer(&blank, &json!(["北京，解析：北京是首都"])),
            None
        );
        let parsed = Html::parse_fragment("<div>3.14 + x</div>");
        assert_eq!(
            body(first(parsed.root_element(), "div").unwrap(), false).unwrap(),
            "3.14 + x"
        );
    }
}
