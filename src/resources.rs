use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use reqwest::{Url, blocking::Client, redirect::Policy};
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};

use crate::model::Question;
use crate::questions::{PageKind, normalize_text, parse_page};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub question_id: Option<u64>,
    pub kind: String,
    pub context: String,
    pub source: Option<String>,
    pub content: Option<String>,
    pub alt: Option<String>,
    pub local_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceManifest {
    pub resources: Vec<Resource>,
}

#[derive(Debug, Serialize)]
pub struct ReviewedQuestion {
    pub index: usize,
    pub question_id: Option<u64>,
    pub question: Option<Question>,
    pub submitted_answer: Option<String>,
    pub reference_answer: Option<String>,
    pub score: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReviewedWork {
    pub title: String,
    pub status: String,
    pub questions: Vec<ReviewedQuestion>,
    pub parse_errors: BTreeMap<usize, String>,
    pub resources: ResourceManifest,
}

fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("static selector")
}

fn question_node(node: ElementRef<'_>) -> Option<ElementRef<'_>> {
    node.ancestors().filter_map(ElementRef::wrap).find(|el| {
        el.value()
            .has_class("Py-mian1", scraper::CaseSensitivity::CaseSensitive)
            || el
                .value()
                .has_class("questionWrap", scraper::CaseSensitivity::CaseSensitive)
    })
}

fn question_id(node: ElementRef<'_>) -> Option<u64> {
    let mut ids = BTreeSet::new();
    if let Some(id) = node
        .value()
        .attr("data")
        .and_then(|id| id.parse::<u64>().ok())
    {
        ids.insert(id);
    }
    for input in node.select(&selector("input")) {
        let value = input.value();
        let id = if value.attr("name") == Some("questionId") {
            value.attr("value")
        } else {
            value
                .attr("id")
                .and_then(|id| id.strip_prefix("answertype"))
        };
        if let Some(id) = id.and_then(|id| id.parse::<u64>().ok()) {
            ids.insert(id);
        }
    }
    if ids.len() == 1 {
        ids.into_iter().find(|id| *id > 0)
    } else {
        None
    }
}

/// Sidecar resources leave the legacy QuestionSet wire format unchanged.
pub fn extract(html: &str, base_url: &str) -> Result<ResourceManifest> {
    let base = Url::parse(base_url).context("资源基准 URL 无效")?;
    let document = Html::parse_document(html);
    let mut resources = Vec::new();
    for node in document.select(&selector("img, math, script[type^='math/tex']")) {
        let owner = question_node(node);
        // Page logos and tracking images are not question resources.
        if owner.is_none() {
            continue;
        }
        let context = node
            .ancestors()
            .filter_map(ElementRef::wrap)
            .find_map(|el| {
                if el
                    .value()
                    .has_class("more-choose-item", scraper::CaseSensitivity::CaseSensitive)
                {
                    el.select(&selector("em.choose-opt"))
                        .next()
                        .and_then(|label| label.value().attr("id-param"))
                        .map(|label| format!("option:{label}"))
                } else if el
                    .value()
                    .has_class("answerList", scraper::CaseSensitivity::CaseSensitive)
                {
                    el.value()
                        .attr("name")
                        .map(|label| format!("option:{label}"))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "body".into());
        let name = node.value().name();
        let source = if name == "img" {
            node.value()
                .attr("src")
                .filter(|src| !src.trim().is_empty())
                .or_else(|| node.value().attr("data-src"))
                .map(|src| {
                    base.join(src)
                        .map(String::from)
                        .unwrap_or_else(|_| src.to_string())
                })
        } else {
            None
        };
        resources.push(Resource {
            question_id: owner.and_then(question_id),
            kind: match name {
                "img" => "image",
                "math" => "mathml",
                _ => "tex",
            }
            .into(),
            context,
            source,
            content: match name {
                "math" => Some(node.html()),
                "script" => Some(node.text().collect()),
                _ => None,
            },
            alt: node.value().attr("alt").map(str::to_owned),
            local_file: None,
        });
    }
    Ok(ResourceManifest { resources })
}

fn trusted_resource_url(raw: &str) -> Result<Url> {
    let url = Url::parse(raw).context("资源 URL 无效")?;
    let host = url.host_str().unwrap_or_default();
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443)
            && ["chaoxing.com", "chaoxing.com.cn"]
                .iter()
                .any(|domain| host == *domain || host.ends_with(&format!(".{domain}"))),
        "资源下载仅允许可信超星 HTTPS 主机"
    );
    Ok(url)
}

/// Explicit opt-in only: this client has no account cookies and follows no redirects.
pub fn download(manifest: &mut ResourceManifest, directory: &Path) -> Result<()> {
    const MAX_BYTES: u64 = 10 * 1024 * 1024;
    ensure!(manifest.resources.len() <= 1000, "资源数量超过下载上限");
    let urls: Vec<_> = manifest
        .resources
        .iter()
        .filter(|resource| resource.kind == "image")
        .map(|resource| trusted_resource_url(resource.source.as_deref().context("图片没有 URL")?))
        .collect::<Result<_>>()?;
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(Policy::none())
        .build()?;
    fs::create_dir_all(directory)?;
    for ((index, resource), url) in manifest
        .resources
        .iter_mut()
        .enumerate()
        .filter(|(_, r)| r.kind == "image")
        .zip(urls)
    {
        let response = client
            .get(url)
            .send()
            .map_err(|_| anyhow::anyhow!("资源下载请求失败"))?;
        ensure!(response.status().is_success(), "资源下载未返回成功状态");
        let media_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        let extension = match media_type {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "image/svg+xml" => "svg",
            _ => anyhow::bail!("资源响应不是受支持图片"),
        };
        ensure!(
            response
                .content_length()
                .is_none_or(|size| size <= MAX_BYTES),
            "图片超过 10 MiB 上限"
        );
        let mut body = Vec::new();
        response.take(MAX_BYTES + 1).read_to_end(&mut body)?;
        ensure!(body.len() as u64 <= MAX_BYTES, "图片超过 10 MiB 上限");
        let name = format!("resource-{index}.{extension}");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(&name))?;
        file.write_all(&body)?;
        resource.local_file = Some(name);
    }
    Ok(())
}

fn labeled(node: ElementRef<'_>, labels: &[&str]) -> Option<String> {
    let mut values = BTreeSet::new();
    for field in node.select(&selector("p, div, span, dd, li")) {
        // A field must be a standalone label/value element, not an entire explanation section.
        if field.select(&selector("p, div, dd, li")).next().is_some() {
            continue;
        }
        let in_explanation = field
            .ancestors()
            .filter_map(ElementRef::wrap)
            .take_while(|ancestor| *ancestor != node)
            .any(|ancestor| {
                let text = normalize_text(&ancestor.text().collect::<String>());
                ["解析", "答案解析", "分析"].iter().any(|label| {
                    text.strip_prefix(label)
                        .is_some_and(|tail| tail.trim_start().starts_with([':', '：']))
                })
            });
        if in_explanation {
            continue;
        }
        let text = normalize_text(&field.text().collect::<String>());
        for label in labels {
            if let Some(tail) = text.strip_prefix(label)
                && let Some(value) = tail.trim_start().strip_prefix([':', '：'])
            {
                let value = value.trim();
                if !value.is_empty()
                    && !["解析：", "解析:", "分析：", "分析:"]
                        .iter()
                        .any(|marker| value.contains(marker))
                {
                    values.insert(value.to_owned());
                }
            }
        }
    }
    if values.len() == 1 {
        values.into_iter().next()
    } else {
        None
    }
}

pub fn parse_review(html: &str, base_url: &str) -> Result<ReviewedWork> {
    let document = Html::parse_document(html);
    ensure!(
        document
            .select(&selector("title"))
            .any(|title| title.text().collect::<String>().contains("已批阅")),
        "页面尚未确认已批阅"
    );
    let paper = parse_page(html, PageKind::Work)?;
    let mut questions = Vec::new();
    for (index, node) in document.select(&selector("div.Py-mian1")).enumerate() {
        let id = question_id(node);
        let mut question = paper
            .indices
            .iter()
            .position(|i| *i == index)
            .map(|position| paper.questions[position].clone());
        // Review fields must never be reused as a new submission candidate.
        if let Some(question) = &mut question {
            question.answer = serde_json::Value::Null;
        }
        questions.push(ReviewedQuestion {
            index,
            question_id: id,
            question,
            submitted_answer: labeled(node, &["我的答案", "学生答案"]),
            reference_answer: labeled(node, &["正确答案", "参考答案"]),
            score: labeled(node, &["得分", "本题得分"]),
        });
    }
    ensure!(!questions.is_empty(), "批阅页面缺少可识别题目节点");
    Ok(ReviewedWork {
        title: paper.title,
        status: "reviewed".into(),
        questions,
        parse_errors: paper.parse_errors,
        resources: extract(html, base_url)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const PAGE: &str = r#"<html><title>作业已批阅</title><h3 class="py-Title">批阅示例</h3><input id="totalQuestionNum" value="1"><img src="/logo.png"><div class="Py-mian1" data="42"><input id="answertype42" value="4"><div class="Py-m1-title">解释<img src="/formula.png" alt="公式"><math><mi>x</mi><mo>+</mo><mn>1</mn></math><script type="math/tex">x+1</script></div><p>我的答案：未知</p><p>正确答案：分情况讨论</p><p>得分：2.5</p><p>解析：<span>正确答案：不要提取这句</span></p></div></html>"#;

    #[test]
    fn resources_keep_formula_and_images_without_modifying_answers() {
        let manifest = extract(PAGE, "https://mooc1.chaoxing.com/work/review").unwrap();
        assert_eq!(manifest.resources.len(), 3);
        assert_eq!(manifest.resources[0].question_id, Some(42));
        assert_eq!(
            manifest.resources[0].source.as_deref(),
            Some("https://mooc1.chaoxing.com/formula.png")
        );
        assert!(
            manifest.resources[1]
                .content
                .as_ref()
                .unwrap()
                .contains("<mi>x</mi>")
        );
        assert_eq!(manifest.resources[2].content.as_deref(), Some("x+1"));
        let review = parse_review(PAGE, "https://mooc1.chaoxing.com/").unwrap();
        let question = &review.questions[0];
        assert_eq!(question.reference_answer.as_deref(), Some("分情况讨论"));
        assert_eq!(question.submitted_answer.as_deref(), Some("未知"));
        assert_eq!(question.score.as_deref(), Some("2.5"));
        assert!(question.question.as_ref().unwrap().answer.is_null());
        assert!(!question.question.as_ref().unwrap().kind.supported());
    }

    #[test]
    fn preserves_option_placement_and_refuses_conflicting_question_ids() {
        let page = r#"<div class="Py-mian1" data="42"><input id="answertype43" value="0"><li class="more-choose-item"><em class="choose-opt" id-param="B">B.</em><img data-src="//p.ananas.chaoxing.com/b.png"></li></div>"#;
        let resources = extract(page, "https://mooc1.chaoxing.com/").unwrap();
        assert_eq!(resources.resources[0].context, "option:B");
        assert_eq!(resources.resources[0].question_id, None);
        assert_eq!(
            resources.resources[0].source.as_deref(),
            Some("https://p.ananas.chaoxing.com/b.png")
        );
        let mut unsafe_manifest = extract(
            &PAGE.replace("/formula.png", "https://evil.test/secret"),
            "https://mooc1.chaoxing.com/",
        )
        .unwrap();
        assert!(download(&mut unsafe_manifest, &std::env::temp_dir()).is_err());
        assert!(
            unsafe_manifest
                .resources
                .iter()
                .all(|resource| resource.local_file.is_none())
        );
    }

    #[test]
    fn rejects_untrusted_sources_and_ambiguous_review_values() {
        for url in [
            "http://chaoxing.com/image.png",
            "https://chaoxing.com.evil.test/image",
            "https://127.0.0.1/image",
            "https://user:secret@chaoxing.com/image",
            "https://chaoxing.com:444/image",
            "file:///tmp/image",
            "data:image/png;base64,AA",
        ] {
            assert!(trusted_resource_url(url).is_err(), "{url}");
        }
        assert!(trusted_resource_url("https://p.ananas.chaoxing.com/image.png").is_ok());
        let page = PAGE.replace("<p>得分：", "<p>正确答案：另一个答案</p><p>得分：");
        assert!(
            parse_review(&page, "https://mooc1.chaoxing.com/")
                .unwrap()
                .questions[0]
                .reference_answer
                .is_none()
        );
        assert!(
            parse_review(
                &PAGE.replace("作业已批阅", "作业"),
                "https://mooc1.chaoxing.com/"
            )
            .is_err()
        );
    }
}
