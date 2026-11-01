//! The cleanup contract: the transcript is an instruction to an agent, and a tidy one
//! that says something the reader did not say makes the agent act on a fiction. Every
//! rule here is deterministic and has a fixture; the model-driven rewrites live in
//! `super::llm` and go through [`guard`] before they are believed.
use crate::api::Repair;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaned {
    pub text: String,
    pub repairs: Vec<Repair>,
}

/// Sounds, not words: dropped wherever they are.
const FILLERS: &[&str] = &["um", "uh", "uhm", "erm", "hmm", "mhm", "mm"];
/// Words that are fillers only when the recogniser set them off as their own clause.
/// "I like this" keeps its "like".
const CLAUSE_FILLERS: &[&str] = &["like", "you know", "i mean", "sort of", "kind of"];
/// Throat-clearing at the start of a sentence: "so um drop the cache column".
const OPENERS: &[&str] = &["so", "well", "okay", "ok", "right", "yeah"];
/// What a speaker says before replacing what they just said. "or" is deliberately not
/// here: "the second, or the third?" is a question, and guessing a side is refused.
const CORRECTIONS: &[&str] = &[
    "no wait",
    "wait no",
    "scratch that",
    "or rather",
    "actually no",
    "i mean",
    "sorry",
    "no",
];
/// Spoken punctuation inside an identifier: "eink underscore token".
const SPOKEN_PUNCTUATION: &[&str] = &["underscore", "dot", "dash", "hyphen", "slash", "point"];
/// Words that carry no content, so a rewrite may add or drop them freely.
const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "am", "to", "of", "in",
    "on", "at", "by", "for", "with", "and", "or", "but", "it", "its", "this", "that", "these",
    "those", "there", "here", "then", "than", "so", "as", "if", "we", "you", "i", "me", "my",
    "our", "your", "they", "them", "their", "he", "she", "his", "her", "do", "does", "did",
    "should", "could", "would", "can", "will", "shall", "may", "might", "what", "why", "how",
    "when", "where", "which", "who", "not", "no", "yes", "please", "just", "also", "too", "very",
    "really", "um", "uh", "like", "mean", "know", "well", "okay", "ok", "one", "ones", "thing",
    "things", "into", "from", "up", "down", "out", "about", "over", "have", "has", "had", "get",
    "got", "make", "made", "let", "lets", "s", "t", "re", "ve", "ll", "d", "m",
];

/// The identifier-like words of a document: what a spoken note can be matched against.
/// Prose words are never terms, so "cash column" stays "cash column" rather than being
/// guessed into "cache column".
pub fn vocabulary(content: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut terms = Vec::new();
    let mut add = |term: &str| {
        let term = term.trim_matches(|c: char| !c.is_alphanumeric());
        if is_term_of_art(term) && seen.insert(term.to_string()) {
            terms.push(term.to_string());
        }
    };
    let mut rest = content;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else { break };
        let span = &after[..close];
        if !span.contains(char::is_whitespace) {
            add(span);
        }
        rest = &after[close + 1..];
    }
    for word in content.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | ':' | '"' | '\'' | '`'
            )
    }) {
        add(word);
    }
    terms.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    terms.truncate(500);
    terms
}

/// A term of art is spelled in a way prose never is: an underscore, a file extension, a
/// case change inside the word, or a digit beside letters. A plain capitalised word or
/// an acronym is not one.
fn is_term_of_art(word: &str) -> bool {
    if word.len() < 3 || word.len() > 40 || !word.chars().any(char::is_alphabetic) {
        return false;
    }
    let alnum = alnum(word);
    if alnum.len() < 4 {
        return false;
    }
    let has_underscore = word.contains('_');
    let has_extension = word.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (2..=5).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_lowercase())
    });
    let chars: Vec<char> = word.chars().collect();
    let inner_case_change = chars
        .windows(2)
        .any(|w| w[0].is_lowercase() && w[1].is_uppercase());
    let digit_beside_letter = chars.windows(2).any(|w| {
        (w[0].is_ascii_digit() && w[1].is_alphabetic())
            || (w[0].is_alphabetic() && w[1].is_ascii_digit())
    });
    let path_like = word.contains('/') && word.chars().filter(|c| *c == '/').count() <= 4;
    has_underscore || has_extension || inner_case_change || digit_beside_letter || path_like
}

fn alnum(word: &str) -> String {
    word.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

/// Rules 2 to 5: fillers go, a self-correction keeps the correction, an ambiguous one
/// keeps both, identifiers take the document's spelling. Rules 6 and 7 (hedges stay, no
/// summarising) hold because nothing here knows what a hedge or a summary is.
pub fn clean(raw: &str, vocabulary: &[String]) -> Cleaned {
    let mut repairs = Vec::new();
    let mut out = Vec::new();
    for (sentence, terminator) in sentences_of(&normalise_dashes(raw)) {
        let mut clauses: Vec<Vec<String>> = sentence
            .split(',')
            .map(|clause| clause.split_whitespace().map(str::to_string).collect())
            .filter(|words: &Vec<String>| !words.is_empty())
            .collect();
        apply_corrections(&mut clauses);
        strip_fillers(&mut clauses);
        for clause in &mut clauses {
            repair(clause, vocabulary, &mut repairs);
        }
        let clauses: Vec<String> = clauses
            .into_iter()
            .filter(|c| !c.is_empty())
            .map(|c| c.join(" "))
            .collect();
        if clauses.is_empty() {
            continue;
        }
        let sentence = clauses.join(", ");
        // A sentence that opens with an identifier keeps its spelling: "reanchor.rs
        // refuses" is not "Reanchor.rs refuses".
        let opens_with_term = sentence.split_whitespace().next().is_some_and(|first| {
            vocabulary
                .iter()
                .any(|t| first.trim_end_matches(|c: char| !c.is_alphanumeric()) == t)
        });
        let sentence = if opens_with_term {
            sentence
        } else {
            capitalise(&sentence)
        };
        out.push(format!("{sentence}{terminator}"));
    }
    Cleaned {
        text: out.join(" "),
        repairs,
    }
}

fn normalise_dashes(raw: &str) -> String {
    raw.replace(" — ", ", ")
        .replace(" – ", ", ")
        .replace(" - ", ", ")
        .replace("—", ", ")
}

/// Sentences with the mark that ended each, so a question stays a question. A mark ends a
/// sentence only when whitespace or the end follows it: the dot in `reanchor.rs` is not
/// the end of anything.
fn sentences_of(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let ends = matches!(c, '.' | '!' | '?')
            && chars.get(i + 1).is_none_or(|next| next.is_whitespace());
        if ends {
            if !current.trim().is_empty() {
                out.push((current.trim().to_string(), c.to_string()));
            }
            current.clear();
        } else {
            current.push(c);
        }
    }
    if !current.trim().is_empty() {
        out.push((current.trim().to_string(), ".".into()));
    }
    out
}

/// Rule 3. A clause that opens with a correction marker replaces the clause before it,
/// from the first word the two share: "the second one, no, the third" keeps "the third";
/// "drop the cache column, no wait, the index column" keeps "drop the index column".
/// A marker with nothing after it ("no," on its own) is only a filler.
fn apply_corrections(clauses: &mut Vec<Vec<String>>) {
    let mut i = 1;
    while i < clauses.len() {
        let Some(marker_len) = correction_marker(&clauses[i]) else {
            i += 1;
            continue;
        };
        let after: Vec<String> = clauses[i][marker_len..].to_vec();
        clauses.remove(i);
        if after.is_empty() {
            if i < clauses.len() {
                let next = clauses.remove(i);
                replace_with(&mut clauses[i - 1], next);
            }
            continue;
        }
        replace_with(&mut clauses[i - 1], after);
    }
}

fn replace_with(before: &mut Vec<String>, after: Vec<String>) {
    let first = alnum(&after[0]);
    let keep = before.iter().position(|w| alnum(w) == first).unwrap_or(0);
    before.truncate(keep);
    before.extend(after);
}

fn correction_marker(clause: &[String]) -> Option<usize> {
    let lowered: Vec<String> = clause.iter().map(|w| alnum(w)).collect();
    CORRECTIONS.iter().find_map(|marker| {
        let words: Vec<&str> = marker.split(' ').collect();
        (lowered.len() >= words.len() && lowered[..words.len()] == words[..]).then_some(words.len())
    })
}

/// Rule 2.
fn strip_fillers(clauses: &mut [Vec<String>]) {
    let mut first_word = true;
    for clause in clauses.iter_mut() {
        clause.retain(|w| !FILLERS.contains(&alnum(w).as_str()));
        if first_word && !clause.is_empty() && OPENERS.contains(&alnum(&clause[0]).as_str()) {
            clause.remove(0);
        }
        if !clause.is_empty() {
            first_word = false;
        }
        for filler in CLAUSE_FILLERS {
            let words: Vec<&str> = filler.split(' ').collect();
            if clause.len() >= words.len()
                && clause[..words.len()]
                    .iter()
                    .zip(&words)
                    .all(|(w, f)| alnum(w) == *f)
            {
                clause.drain(..words.len());
            }
        }
    }
}

/// Rule 5, span-local and narrow: a window of up to four spoken words is rewritten only
/// when, with its punctuation and the spoken "dot"/"underscore" removed, it is exactly
/// a term the document uses. Anything looser once turned "the submit path" into "the
/// submit" and `EINK_TOKEN` into `EINK_TOKEN_Token`.
fn repair(clause: &mut Vec<String>, vocabulary: &[String], repairs: &mut Vec<Repair>) {
    let mut i = 0;
    while i < clause.len() {
        let mut matched = None;
        for width in (1..=4.min(clause.len() - i)).rev() {
            let window = &clause[i..i + width];
            let spoken: String = window
                .iter()
                .filter(|w| !SPOKEN_PUNCTUATION.contains(&alnum(w).as_str()))
                .map(|w| alnum(w))
                .collect();
            if spoken.len() < 4 {
                continue;
            }
            if let Some(term) = vocabulary.iter().find(|term| alnum(term) == spoken) {
                matched = Some((width, term.clone()));
                break;
            }
        }
        match matched {
            Some((width, term)) => {
                let heard = clause[i..i + width].join(" ");
                let trailing: String = clause[i + width - 1]
                    .chars()
                    .rev()
                    .take_while(|c| !c.is_alphanumeric())
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                if heard.trim_end_matches(trailing.as_str()) != term {
                    repairs.push(Repair {
                        heard: heard.trim_end_matches(trailing.as_str()).to_string(),
                        wrote: term.clone(),
                    });
                }
                clause.splice(i..i + width, [format!("{term}{trailing}")]);
                i += 1;
            }
            None => i += 1,
        }
    }
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The content words of a note: everything that is not a stop word, with each
/// whitespace-delimited word also kept whole without its punctuation, so `reanchor.rs`
/// and "re-anchor rs" meet in the middle.
pub fn content_words(text: &str) -> HashSet<String> {
    let mut words = HashSet::new();
    for word in text.split_whitespace() {
        let whole = alnum(word);
        if whole.len() >= 2 && !STOP_WORDS.contains(&whole.as_str()) {
            words.insert(whole);
        }
        for part in word.split(|c: char| !c.is_ascii_alphanumeric()) {
            let part = part.to_ascii_lowercase();
            if part.len() >= 2 && !STOP_WORDS.contains(&part.as_str()) {
                words.insert(part);
            }
        }
    }
    words
}

/// Rule 1, checked rather than trusted: every content word of the cleaned note must
/// appear in the raw, modulo the repairs and the stop words. A cleaner that turns "add a
/// cache column" into "add a caching layer with Redis" is refused here, and the raw wins.
pub fn guard(raw: &str, cleaned: &str, repairs: &[Repair]) -> bool {
    let mut allowed = content_words(raw);
    for repair in repairs {
        allowed.extend(content_words(&repair.wrote));
        allowed.insert(alnum(&repair.wrote));
    }
    content_words(cleaned)
        .iter()
        .all(|word| allowed.contains(word))
}

/// What the reader can say to the note once it is on the card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verb {
    Shorter,
    Longer,
    /// 1-based.
    Drop(usize),
    DropLast,
    List,
    Question,
    Blocker,
    Undo,
    Done,
    /// Anything outside the closed set: previewed, never applied silently.
    Free(String),
}

pub fn parse_verb(instruction: &str) -> Verb {
    let words: Vec<String> = instruction
        .split_whitespace()
        .map(alnum)
        .filter(|w| !w.is_empty())
        .collect();
    let joined = words.join(" ");
    match joined.as_str() {
        "shorter" | "make it shorter" => Verb::Shorter,
        "longer" | "make it longer" => Verb::Longer,
        "undo" => Verb::Undo,
        "done" | "send" => Verb::Done,
        "turn that into a list"
        | "as a list"
        | "make it a list"
        | "turn it into a list"
        | "list" => Verb::List,
        "make it a question" | "as a question" | "question" => Verb::Question,
        "mark it a blocker"
        | "mark as blocker"
        | "mark it as a blocker"
        | "blocker"
        | "this is a blocker"
        | "it is a blocker" => Verb::Blocker,
        "drop the last one" | "drop the last" | "drop last" => Verb::DropLast,
        _ => {
            if words.len() == 2
                && words[0] == "drop"
                && let Some(n) = ordinal(&words[1])
            {
                return Verb::Drop(n);
            }
            Verb::Free(instruction.trim().to_string())
        }
    }
}

fn ordinal(word: &str) -> Option<usize> {
    const NAMES: [&str; 10] = [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    ];
    if let Ok(n) = word.parse::<usize>() {
        return (n >= 1).then_some(n);
    }
    NAMES.iter().position(|n| *n == word).map(|i| i + 1)
}

/// The sentences of a note. A note already shaped as a list is one item per line.
pub fn sentences(text: &str) -> Vec<String> {
    if text.lines().count() > 1 && text.lines().all(|l| l.trim_start().starts_with("- ")) {
        return text
            .lines()
            .map(|l| l.trim_start().trim_start_matches("- ").to_string())
            .collect();
    }
    sentences_of(text)
        .into_iter()
        .map(|(s, t)| format!("{s}{t}"))
        .collect()
}

/// Sentence `n` (1-based) removed, or nothing to remove.
pub fn drop_sentence(text: &str, n: usize) -> Option<String> {
    let mut parts = sentences(text);
    if n == 0 || n > parts.len() {
        return None;
    }
    parts.remove(n - 1);
    Some(join_like(text, &parts))
}

pub fn as_list(text: &str) -> String {
    sentences(text)
        .iter()
        .map(|s| format!("- {s}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn join_like(original: &str, parts: &[String]) -> String {
    if original.lines().count() > 1 && original.lines().all(|l| l.trim_start().starts_with("- ")) {
        parts
            .iter()
            .map(|s| format!("- {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab() -> Vec<String> {
        vocabulary(
            "The server keeps `reanchor.rs` and `lasso_bbox` honest. Set EINK_TOKEN for the LAN. \
             The WebSocket carries it; `submit_review` and `ws.rs` read it. A cache column is slow.",
        )
    }

    #[test]
    fn a_vocabulary_is_identifiers_not_prose() {
        let v = vocab();
        for term in [
            "reanchor.rs",
            "lasso_bbox",
            "EINK_TOKEN",
            "WebSocket",
            "submit_review",
            "ws.rs",
        ] {
            assert!(v.contains(&term.to_string()), "{term} is a term of art");
        }
        for word in ["cache", "column", "The", "LAN", "server", "honest"] {
            assert!(
                !v.contains(&word.to_string()),
                "{word} is prose or an acronym"
            );
        }
    }

    #[test]
    fn rule_2_filler_goes() {
        let c = clean(
            "so um drop the cache column, you know, the index is enough",
            &[],
        );
        assert_eq!(c.text, "Drop the cache column, the index is enough.");
        assert!(c.repairs.is_empty());
        assert_eq!(
            clean("I like this, uh, a lot.", &[]).text,
            "I like this, a lot."
        );
    }

    #[test]
    fn rule_3_a_self_correction_keeps_the_correction() {
        assert_eq!(
            clean("the second one — no, the third", &[]).text,
            "The third."
        );
        assert_eq!(
            clean("drop the cache column, no wait, the index column", &[]).text,
            "Drop the index column."
        );
        assert_eq!(
            clean("use a hash, sorry, use a set here", &[]).text,
            "Use a set here."
        );
    }

    #[test]
    fn rule_4_an_ambiguous_correction_keeps_both_in_order() {
        assert_eq!(
            clean("move it to the second, or the third?", &[]).text,
            "Move it to the second, or the third?"
        );
    }

    #[test]
    fn rule_5_identifiers_take_the_documents_spelling() {
        let c = clean(
            "re-anchor dot rs should refuse this, and eink underscore token too",
            &vocab(),
        );
        assert_eq!(
            c.text,
            "reanchor.rs should refuse this, and EINK_TOKEN too."
        );
        assert_eq!(
            c.repairs,
            vec![
                Repair {
                    heard: "re-anchor dot rs".into(),
                    wrote: "reanchor.rs".into()
                },
                Repair {
                    heard: "eink underscore token".into(),
                    wrote: "EINK_TOKEN".into()
                },
            ]
        );
        let c = clean("the web socket drops, lasso b-box too.", &vocab());
        assert_eq!(c.text, "The WebSocket drops, lasso_bbox too.");
    }

    #[test]
    fn rule_5_is_narrow_prose_is_left_alone() {
        // "cash column" is one edit from "cache column" and stays wrong rather than guessed.
        let c = clean("the cash column is slow", &vocab());
        assert_eq!(c.text, "The cash column is slow.");
        assert!(c.repairs.is_empty());
        // A term spoken as itself is not a repair.
        let c = clean("EINK_TOKEN is read twice", &vocab());
        assert_eq!(c.text, "EINK_TOKEN is read twice.");
        assert!(c.repairs.is_empty());
        // The submit path is prose, even though submit_review is a term.
        let c = clean("the submit path is fine", &vocab());
        assert_eq!(c.text, "The submit path is fine.");
    }

    #[test]
    fn a_dot_inside_an_identifier_is_not_the_end_of_a_sentence() {
        // Whisper spells a spoken file name right more often than not; the cleaner must
        // not then cut it in two.
        let c = clean(
            "so um, reanchor.rs should refuse this. the index is enough",
            &vocab(),
        );
        assert_eq!(
            c.text,
            "reanchor.rs should refuse this. The index is enough."
        );
        assert!(c.repairs.is_empty());
        assert_eq!(
            sentences("reanchor.rs refuses. ws.rs reads."),
            vec!["reanchor.rs refuses.", "ws.rs reads."]
        );
    }

    #[test]
    fn rule_6_hedges_stay() {
        let c = clean("I think maybe this is wrong, not sure though", &[]);
        assert_eq!(c.text, "I think maybe this is wrong, not sure though.");
    }

    #[test]
    fn rule_7_nothing_is_summarised_or_reordered() {
        let raw = "first drop the cache column. then add an index on user id. finally run the migration before the deploy.";
        let c = clean(raw, &[]);
        assert_eq!(
            c.text,
            "First drop the cache column. Then add an index on user id. Finally run the migration before the deploy."
        );
        assert!(content_words(raw).is_subset(&content_words(&c.text)));
    }

    #[test]
    fn the_guard_refuses_an_invented_word_and_allows_a_reorder() {
        assert!(!guard(
            "add a cache column",
            "add a caching layer with Redis",
            &[]
        ));
        assert!(guard(
            "add a cache column, then the index",
            "the index, then add a cache column",
            &[]
        ));
        assert!(guard(
            "re-anchor rs refuses",
            "reanchor.rs refuses",
            &[Repair {
                heard: "re-anchor rs".into(),
                wrote: "reanchor.rs".into()
            }]
        ));
        assert!(
            !guard("re-anchor rs refuses", "reanchor.rs refuses", &[]),
            "a spelling the raw never had needs a repair to vouch for it"
        );
        assert!(
            guard("drop the column", "should we drop the column?", &[]),
            "question words are not content"
        );
        assert!(guard(
            "drop the cache column. the index is enough.",
            "- drop the cache column.\n- the index is enough.",
            &[]
        ));
    }

    #[test]
    fn the_closed_verbs_parse_and_anything_else_is_free() {
        assert_eq!(parse_verb("shorter"), Verb::Shorter);
        assert_eq!(parse_verb("Drop two"), Verb::Drop(2));
        assert_eq!(parse_verb("drop 3"), Verb::Drop(3));
        assert_eq!(parse_verb("drop the last one"), Verb::DropLast);
        assert_eq!(parse_verb("turn that into a list"), Verb::List);
        assert_eq!(parse_verb("mark it a blocker"), Verb::Blocker);
        assert_eq!(parse_verb("make it a question"), Verb::Question);
        assert_eq!(parse_verb("undo"), Verb::Undo);
        assert_eq!(parse_verb("done"), Verb::Done);
        assert_eq!(
            parse_verb("say it more politely"),
            Verb::Free("say it more politely".into())
        );
        assert_eq!(
            parse_verb("drop everything"),
            Verb::Free("drop everything".into())
        );
    }

    #[test]
    fn sentences_can_be_dropped_and_listed() {
        let text = "Drop the cache column. The index is enough. Ship it!";
        assert_eq!(
            drop_sentence(text, 2).unwrap(),
            "Drop the cache column. Ship it!"
        );
        assert_eq!(drop_sentence(text, 4), None);
        assert_eq!(drop_sentence(text, 0), None);
        let list = as_list(text);
        assert_eq!(
            list,
            "- Drop the cache column.\n- The index is enough.\n- Ship it!"
        );
        assert_eq!(
            drop_sentence(&list, 3).unwrap(),
            "- Drop the cache column.\n- The index is enough."
        );
    }
}
