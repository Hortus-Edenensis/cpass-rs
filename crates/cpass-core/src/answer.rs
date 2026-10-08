use std::collections::BTreeSet;
use std::fmt;

use scraper::Html;
use serde::Deserializer;
use serde::de::{MapAccess, Visitor};
use serde_json::Value;

use crate::searcher::{AnswerCandidate, AnswerQuery, AnswerQuestionKind};

pub(crate) fn selected_candidate(
    query: &AnswerQuery,
    candidates: &[AnswerCandidate],
) -> Option<AnswerCandidate> {
    if query.question_kind != AnswerQuestionKind::from_question_type(query.question_type) {
        return None;
    }
    let mut selected: Option<(AnswerCandidate, Value)> = None;
    for candidate in candidates {
        let Some(answer) = normalize_answer(query, &candidate.answer) else {
            continue;
        };
        if let Some((_, previous)) = &selected {
            if previous != &answer {
                return None;
            }
        } else {
            let mut candidate = candidate.clone();
            candidate.answer = match &answer {
                Value::String(text) => text.clone(),
                _ => answer.to_string(),
            };
            selected = Some((candidate, answer));
        }
    }
    selected.map(|(candidate, _)| candidate)
}

fn normalize_answer(query: &AnswerQuery, raw: &str) -> Option<Value> {
    let raw = final_content(raw)?;
    let decoded = answer_field(raw).and_then(|answer| normalize_decoded(query, &answer));
    if matches!(
        query.question_kind,
        AnswerQuestionKind::SingleChoice | AnswerQuestionKind::MultipleChoice
    ) && raw.starts_with(['[', '{'])
        && let Some(literal) = choice(query, raw)
    {
        let literal = Value::String(literal);
        return (decoded.is_none() || decoded.as_ref() == Some(&literal)).then_some(literal);
    }
    decoded
}

fn final_content(raw: &str) -> Option<&str> {
    let mut text = raw.trim();
    if text.to_ascii_lowercase().starts_with("<think>") {
        let lower = text.to_ascii_lowercase();
        let end = lower.find("</think>")?;
        if lower[7..end].contains("<think") {
            return None;
        }
        text = text[end + 8..].trim();
    }
    if has_thinking(text) {
        return None;
    }
    if text.starts_with("```") {
        let (opening, body) = text.split_once('\n')?;
        if !matches!(
            opening.trim().to_ascii_lowercase().as_str(),
            "```" | "```json" | "```text"
        ) {
            return None;
        }
        text = body.strip_suffix("```")?.trim();
    }
    (!text.is_empty() && !text.contains("```")).then_some(text)
}

fn has_thinking(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("<think") || lower.contains("</think")
}

fn answer_field(raw: &str) -> Option<Value> {
    let mut answer = if raw.starts_with('{') {
        let mut decoder = serde_json::Deserializer::from_str(raw);
        let fields = decoder.deserialize_map(UniqueFields).ok()?;
        decoder.end().ok()?;
        fields
    } else if raw.starts_with(['[', '"']) {
        serde_json::from_str(raw).ok()?
    } else {
        Value::String(raw.to_owned())
    };
    if let Value::Object(fields) = &answer {
        let mut values = ["answer", "answers", "答案"]
            .into_iter()
            .filter_map(|key| fields.get(key));
        let value = values.next()?.clone();
        if values.next().is_some() {
            return None;
        }
        answer = value;
    }
    if let Value::String(raw) = &answer {
        if has_thinking(raw) || raw.contains("```") {
            return None;
        }
        let lines = raw.lines().collect::<Vec<_>>();
        let end = lines
            .iter()
            .position(|line| analysis_marker(line.trim_start()).is_some())
            .unwrap_or(lines.len());
        let text = lines[..end].join("\n");
        let text = strip_answer_prefix(&text).trim();
        if has_analysis(text) {
            return None;
        }
        answer = Value::String(text.to_owned());
    }
    Some(answer)
}

struct UniqueFields;

impl<'de> Visitor<'de> for UniqueFields {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unique answer fields")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Value, M::Error> {
        let mut fields = serde_json::Map::new();
        while let Some((key, value)) = map.next_entry::<String, Value>()? {
            if fields.insert(key, value).is_some() {
                return Err(serde::de::Error::custom("duplicate answer field"));
            }
        }
        Ok(Value::Object(fields))
    }
}

fn strip_answer_prefix(text: &str) -> &str {
    let text = text.trim_start();
    for prefix in ["参考答案", "正确答案", "答案", "answer"] {
        if let Some(head) = text.get(..prefix.len())
            && head.eq_ignore_ascii_case(prefix)
        {
            let tail = text[prefix.len()..].trim_start();
            if let Some(tail) = tail.strip_prefix([':', '：']) {
                return tail;
            }
        }
    }
    text
}

fn analysis_marker(text: &str) -> Option<&str> {
    for marker in [
        "解析",
        "解释",
        "理由",
        "说明",
        "analysis",
        "explanation",
        "reason",
    ] {
        if let Some(head) = text.get(..marker.len())
            && head.eq_ignore_ascii_case(marker)
            && text[marker.len()..].trim_start().starts_with([':', '：'])
        {
            return Some(marker);
        }
    }
    None
}

fn has_analysis(text: &str) -> bool {
    text.char_indices().any(|(index, _)| {
        (index == 0
            || text[..index].ends_with(|ch: char| ch.is_whitespace() || ",，;；。.".contains(ch)))
            && analysis_marker(&text[index..]).is_some()
    })
}

fn normalize_text(text: &str) -> String {
    let escaped = text.replace('<', "&lt;").replace('>', "&gt;");
    let decoded = Html::parse_fragment(&escaped)
        .root_element()
        .text()
        .collect::<String>();
    decoded
        .chars()
        .filter(|ch| {
            !matches!(
                ch,
                '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}'
            )
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn label(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            'Ａ'..='Ｚ' | 'ａ'..='ｚ' => char::from_u32(ch as u32 - 0xfee0)
                .map(|ch| ch.to_ascii_uppercase())
                .unwrap_or(ch),
            _ => ch.to_ascii_uppercase(),
        })
        .collect()
}

fn option_prefix(text: &str) -> Option<(&str, &str)> {
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_ascii_alphabetic() && !matches!(first, 'Ａ'..='Ｚ' | 'ａ'..='ｚ') {
        return None;
    }
    let (index, punctuation) = chars.next()?;
    ".．、:：)）".contains(punctuation).then(|| {
        (
            &text[..index],
            text[index + punctuation.len_utf8()..].trim_start(),
        )
    })
}

fn option_text(key: &str, text: &str) -> String {
    let text = normalize_text(text);
    if let Some((prefix, body)) = option_prefix(&text)
        && label(prefix) == label(key)
    {
        return body.to_owned();
    }
    text
}

fn choice(query: &AnswerQuery, raw: &str) -> Option<String> {
    let text = normalize_text(raw);
    let labels = query
        .options
        .iter()
        .map(|option| label(&option.key))
        .collect::<BTreeSet<_>>();
    if text.is_empty()
        || query.options.is_empty()
        || labels.len() != query.options.len()
        || labels.contains("")
    {
        return None;
    }
    if let Some((key, body)) = option_prefix(&text) {
        let option = query
            .options
            .iter()
            .find(|option| label(&option.key) == label(key))?;
        return (body.is_empty() || body == option_text(&option.key, &option.value))
            .then(|| option.key.clone());
    }
    let mut matches = query.options.iter().filter(|option| {
        label(&option.key) == label(&text) || option_text(&option.key, &option.value) == text
    });
    let option = matches.next()?;
    matches.next().is_none().then(|| option.key.clone())
}

fn normalize_decoded(query: &AnswerQuery, answer: &Value) -> Option<Value> {
    match query.question_kind {
        AnswerQuestionKind::TrueFalse => {
            if let Some(value) = answer.as_bool() {
                return Some(Value::Bool(value));
            }
            let text = match answer {
                Value::Number(number) => number.to_string(),
                Value::String(text) => normalize_text(text)
                    .trim_matches(['。', '.'])
                    .to_ascii_lowercase(),
                _ => return None,
            };
            match text.as_str() {
                "对" | "正确" | "是" | "√" | "true" | "yes" | "1" => Some(Value::Bool(true)),
                "错" | "错误" | "否" | "不正确" | "不对" | "不是" | "×" | "✗" | "false" | "no"
                | "0" => Some(Value::Bool(false)),
                _ => None,
            }
        }
        AnswerQuestionKind::FillBlank => {
            let values = if let Some(text) = answer.as_str() {
                text.split('#')
                    .map(|text| Value::String(text.to_owned()))
                    .collect::<Vec<_>>()
            } else {
                answer.as_array()?.clone()
            };
            if query.blanks.is_empty() || values.len() != query.blanks.len() {
                return None;
            }
            let values = values
                .iter()
                .map(|value| {
                    let text = value.as_str()?;
                    let normalized = normalize_text(text);
                    (!normalized.is_empty() && !has_analysis(text) && !has_thinking(text))
                        .then_some(Value::String(normalized))
                })
                .collect::<Option<Vec<_>>>()?;
            Some(Value::Array(values))
        }
        AnswerQuestionKind::SingleChoice => {
            let text = if let Some(text) = answer.as_str() {
                text
            } else {
                let values = answer.as_array()?;
                if values.len() != 1 {
                    return None;
                }
                values[0].as_str()?
            };
            choice(query, text).map(Value::String)
        }
        AnswerQuestionKind::MultipleChoice => {
            let (whole, parts) = if let Some(text) = answer.as_str() {
                let whole = choice(query, text);
                let mut parts = text
                    .split(['#', ';', '；', ',', '，', '、', '|', '\n'])
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                let compact = label(&text.split_whitespace().collect::<String>());
                if parts.len() == 1
                    && !compact.is_empty()
                    && compact.bytes().all(|byte| byte.is_ascii_uppercase())
                {
                    parts = compact.chars().map(|ch| ch.to_string()).collect();
                }
                (whole, parts)
            } else {
                let parts = answer
                    .as_array()?
                    .iter()
                    .map(|value| value.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()?;
                (None, parts)
            };
            if parts.is_empty() {
                return None;
            }
            let keys = parts
                .iter()
                .map(|part| choice(query, part))
                .collect::<Option<BTreeSet<_>>>();
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
            let canonical = query
                .options
                .iter()
                .filter(|option| keys.contains(&option.key))
                .map(|option| option.key.as_str())
                .collect::<String>();
            (!canonical.is_empty()).then_some(Value::String(canonical))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::selected_candidate;
    use crate::models::ExamQuestionOption;
    use crate::searcher::{AnswerCandidate, AnswerQuery, AnswerQuerySource, AnswerQuestionKind};

    fn query(kind: AnswerQuestionKind) -> AnswerQuery {
        let question_type = match kind {
            AnswerQuestionKind::SingleChoice => 0,
            AnswerQuestionKind::MultipleChoice => 1,
            AnswerQuestionKind::FillBlank => 2,
            AnswerQuestionKind::TrueFalse => 3,
            _ => 99,
        };
        AnswerQuery {
            source: AnswerQuerySource::ChapterWork,
            question_index: 0,
            question_id: 42,
            question_type,
            question_type_label: String::new(),
            question_kind: kind,
            prompt: "题目".into(),
            options: [("B", "乙"), ("A", "甲"), ("C", "丙")]
                .into_iter()
                .map(|(key, value)| ExamQuestionOption {
                    key: key.into(),
                    value: value.into(),
                    rich_content: None,
                })
                .collect(),
            blanks: vec!["第一空".into(), "第二空".into()],
        }
    }

    fn select(query: &AnswerQuery, answers: &[&str]) -> Option<String> {
        selected_candidate(
            query,
            &answers
                .iter()
                .map(|answer| AnswerCandidate {
                    provider: "test".into(),
                    confidence: None,
                    answer: (*answer).into(),
                })
                .collect::<Vec<_>>(),
        )
        .map(|candidate| candidate.answer)
    }

    #[test]
    fn classic_answers_require_complete_unique_agreement() {
        let q = query(AnswerQuestionKind::SingleChoice);
        assert_eq!(select(&q, &["未知", "ａ", "甲", "A. 甲"]), Some("A".into()));
        assert_eq!(select(&q, &["A", "B"]), None);
        for raw in ["A. 乙", "甲解释", "[\"A\",\"B\"]", "{\"answer\":"] {
            assert_eq!(select(&q, &[raw]), None, "{raw}");
        }
        let mut ambiguous = q.clone();
        ambiguous.options[0].value = "甲".into();
        assert_eq!(select(&ambiguous, &["甲"]), None);
        ambiguous.options[0].key = "a".into();
        assert_eq!(select(&ambiguous, &["A"]), None);
        let q = query(AnswerQuestionKind::MultipleChoice);
        assert_eq!(
            select(&q, &["ＡＢＡ", "甲#乙", "[\"A\",\"B\"]"]),
            Some("BA".into())
        );
        for raw in ["A;D", "[\"A\",\"未知\"]", "A#"] {
            assert_eq!(select(&q, &[raw]), None, "{raw}");
        }
        assert_eq!(select(&q, &["AB", "AC"]), None);
    }

    #[test]
    fn false_and_complete_blank_values_survive_wrappers() {
        let q = query(AnswerQuestionKind::TrueFalse);
        assert_eq!(
            select(&q, &["false", "FALSE", "{\"answer\":false}", "答案：错误"]),
            Some("false".into())
        );
        assert_eq!(select(&q, &["false", "true"]), None);
        assert_eq!(select(&q, &["这不正确"]), None);
        let q = query(AnswerQuestionKind::FillBlank);
        assert_eq!(
            select(&q, &[" 已有 # 新 ", "{\"answers\":[\"已有\",\"新\"]}"]),
            Some("[\"已有\",\"新\"]".into())
        );
        for raw in [
            "已有",
            "[\"已有\",\"\"]",
            "[\"已有\",false]",
            "已有#新#多余",
        ] {
            assert_eq!(select(&q, &[raw]), None, "{raw}");
        }
        assert_eq!(select(&q, &["甲#乙", "甲#丙"]), None);
        assert_eq!(
            select(&query(AnswerQuestionKind::Unknown(99)), &["A"]),
            None
        );
    }

    #[test]
    fn only_isolated_final_answers_are_used() {
        let q = query(AnswerQuestionKind::SingleChoice);
        for raw in [
            "答案：B\n解析：计算得到",
            "```json\n{\"answer\":\"B\",\"analysis\":\"计算\"}\n```",
            "```text\n答案：B\n```",
            "<think>计算</think>\n答案：B",
        ] {
            assert_eq!(select(&q, &[raw]), Some("B".into()), "{raw}");
        }
        for raw in [
            "答案：B，解析：计算得到",
            "<think>B",
            "B<think>计算</think>",
            "<think><think>计算</think>B",
            "```json\n{\"answer\":\"B\"}",
            "```json\n{\"answer\":\"B\"}\n``` trailing",
            "{\"answer\":\"A\",\"answers\":\"B\"}",
            "{\"answer\":\"A\",\"answer\":\"B\"}",
        ] {
            assert_eq!(select(&q, &[raw]), None, "{raw}");
        }
        let mut literal = q;
        literal.options[0].value = "[0,1]".into();
        assert_eq!(select(&literal, &["[0,1]"]), Some("B".into()));
        literal.options[0].value = "x^{2}".into();
        literal.options[1].value = "x2".into();
        assert_eq!(select(&literal, &["x^{2}"]), Some("B".into()));
        assert_eq!(select(&literal, &["x2"]), Some("A".into()));
    }
}
