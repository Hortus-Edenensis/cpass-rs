use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "question_type", rename_all = "snake_case")]
pub enum NormalizedQuestionKind {
    SingleChoice,
    MultipleChoice,
    FillBlank,
    TrueFalse,
    ShortAnswer,
    TermExplanation,
    Essay,
    Calculation,
    Other,
    JournalEntry,
    Material,
    Matching,
    Ordering,
    Cloze,
    ReadingComprehension,
    Spoken,
    Listening,
    SharedOption,
    Assessment,
    Unknown(u64),
}

impl NormalizedQuestionKind {
    #[must_use]
    pub fn from_question_type(question_type: u64) -> Self {
        match question_type {
            0 => Self::SingleChoice,
            1 => Self::MultipleChoice,
            2 => Self::FillBlank,
            3 => Self::TrueFalse,
            4 => Self::ShortAnswer,
            5 => Self::TermExplanation,
            6 => Self::Essay,
            7 => Self::Calculation,
            8 => Self::Other,
            9 => Self::JournalEntry,
            10 => Self::Material,
            11 => Self::Matching,
            13 => Self::Ordering,
            14 => Self::Cloze,
            15 => Self::ReadingComprehension,
            18 => Self::Spoken,
            19 => Self::Listening,
            20 => Self::SharedOption,
            21 => Self::Assessment,
            other => Self::Unknown(other),
        }
    }

    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::SingleChoice => "single_choice",
            Self::MultipleChoice => "multiple_choice",
            Self::FillBlank => "fill_blank",
            Self::TrueFalse => "true_false",
            Self::ShortAnswer => "short_answer",
            Self::TermExplanation => "term_explanation",
            Self::Essay => "essay",
            Self::Calculation => "calculation",
            Self::Other => "other",
            Self::JournalEntry => "journal_entry",
            Self::Material => "material",
            Self::Matching => "matching",
            Self::Ordering => "ordering",
            Self::Cloze => "cloze",
            Self::ReadingComprehension => "reading_comprehension",
            Self::Spoken => "spoken",
            Self::Listening => "listening",
            Self::SharedOption => "shared_option",
            Self::Assessment => "assessment",
            Self::Unknown(_) => "unknown",
        }
    }
}
