use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Lgtm,
    Changes,
    Reject,
    Question,
}

impl Verdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Lgtm => "lgtm",
            Verdict::Changes => "changes",
            Verdict::Reject => "reject",
            Verdict::Question => "question",
        }
    }

    /// CLI exit code: 0 = lgtm, 2 = changes, 3 = reject, 4 = question.
    pub fn exit_code(&self) -> i32 {
        match self {
            Verdict::Lgtm => 0,
            Verdict::Changes => 2,
            Verdict::Reject => 3,
            Verdict::Question => 4,
        }
    }
}

/// Parse a verdict tag from the start of typed_notes.
/// Recognises: LGTM, CHANGES, REJECT, QUESTION (case-insensitive).
/// The tag may be followed by ':', '-', space, or end of string.
pub fn parse_verdict(typed_notes: &str) -> Option<Verdict> {
    let upper = typed_notes.trim().to_uppercase();
    let tag = upper.split(|c: char| !c.is_alphabetic()).next()?;
    match tag {
        "LGTM" => Some(Verdict::Lgtm),
        "CHANGES" => Some(Verdict::Changes),
        "REJECT" => Some(Verdict::Reject),
        "QUESTION" => Some(Verdict::Question),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lgtm_variants() {
        assert_eq!(parse_verdict("LGTM"), Some(Verdict::Lgtm));
        assert_eq!(parse_verdict("lgtm: looks good"), Some(Verdict::Lgtm));
        assert_eq!(parse_verdict("LGTM - ship it"), Some(Verdict::Lgtm));
        assert_eq!(parse_verdict("lgtm, nice work"), Some(Verdict::Lgtm));
    }

    #[test]
    fn changes_variants() {
        assert_eq!(parse_verdict("CHANGES"), Some(Verdict::Changes));
        assert_eq!(parse_verdict("changes: needs work"), Some(Verdict::Changes));
        assert_eq!(parse_verdict("Changes needed"), Some(Verdict::Changes));
    }

    #[test]
    fn reject_variants() {
        assert_eq!(parse_verdict("REJECT"), Some(Verdict::Reject));
        assert_eq!(
            parse_verdict("reject: wrong approach"),
            Some(Verdict::Reject)
        );
    }

    #[test]
    fn question_variants() {
        assert_eq!(parse_verdict("QUESTION"), Some(Verdict::Question));
        assert_eq!(
            parse_verdict("question: what does this do?"),
            Some(Verdict::Question)
        );
    }

    #[test]
    fn plain_text_returns_none() {
        assert_eq!(parse_verdict("looks good to me"), None);
        assert_eq!(parse_verdict(""), None);
        assert_eq!(parse_verdict("  "), None);
    }

    #[test]
    fn exit_codes() {
        assert_eq!(Verdict::Lgtm.exit_code(), 0);
        assert_eq!(Verdict::Changes.exit_code(), 2);
        assert_eq!(Verdict::Reject.exit_code(), 3);
        assert_eq!(Verdict::Question.exit_code(), 4);
    }
}
