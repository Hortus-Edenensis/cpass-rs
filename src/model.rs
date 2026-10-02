use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct QuestionType(pub i64);

impl QuestionType {
    pub const SINGLE: Self = Self(0);
    pub const MULTIPLE: Self = Self(1);
    pub const BLANK: Self = Self(2);
    pub const TRUE_FALSE: Self = Self(3);

    pub fn supported(self) -> bool {
        (0..=3).contains(&self.0)
    }

    pub fn name(self) -> &'static str {
        match self.0 {
            0 => "单选题",
            1 => "多选题",
            2 => "填空题",
            3 => "判断题",
            4 => "简答题",
            5 => "名词解释",
            6 => "论述题",
            7 => "计算题",
            9 => "分录题",
            10 => "资料题",
            11 => "连线题",
            13 => "排序题",
            14 => "完型填空",
            15 => "阅读理解",
            18 => "口语题",
            19 => "听力题",
            20 => "共用选项题",
            21 => "测评题",
            _ => "其它",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub id: u64,
    pub value: String,
    #[serde(rename = "type")]
    pub kind: QuestionType,
    #[serde(default)]
    pub options: Value,
    #[serde(default)]
    pub answer: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionSet {
    pub id: Value,
    pub title: String,
    #[serde(rename = "type")]
    pub export_type: u8,
    pub questions: Vec<Question>,
}
