//! Semantic matching is server-owned; the renderer supplies actual layout elements.
//! Never trust positional section IDs as identity, or guess between equal matches.
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Element {
    pub i: usize,
    pub tag: String,
    pub text: String,
}

fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    // Character edit distance handles spelling, punctuation and small insertions.
    let a: Vec<_> = a.chars().collect();
    let b: Vec<_> = b.chars().collect();
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (row[j] + 1)
                .min(above + 1)
                .min(diagonal + usize::from(x != y));
            diagonal = above;
        }
    }
    1.0 - row[b.len()] as f64 / a.len().max(b.len()) as f64
}

pub fn targets(previous: &[Element], current: &[Element]) -> Vec<Option<usize>> {
    // A pure reflow/font-size change has no semantic ambiguity, even for repeated
    // labels. Require the entire sequence to match before trusting order.
    if previous.len() == current.len()
        && previous.iter().zip(current).all(|(a, b)| {
            a.tag.eq_ignore_ascii_case(&b.tag) && normalized(&a.text) == normalized(&b.text)
        })
    {
        return current.iter().map(|e| Some(e.i)).collect();
    }
    let texts: Vec<_> = current.iter().map(|e| normalized(&e.text)).collect();
    let proposed: Vec<_> = previous
        .iter()
        .map(|old| {
            let text = normalized(&old.text);
            if text.is_empty() {
                return None;
            }
            let exact: Vec<_> = current
                .iter()
                .zip(&texts)
                .filter(|(e, t)| e.tag.eq_ignore_ascii_case(&old.tag) && **t == text)
                .collect();
            if !exact.is_empty() {
                return if exact.len() == 1 {
                    Some(exact[0].0.i)
                } else {
                    None
                };
            }
            let mut scores: Vec<_> = current
                .iter()
                .zip(&texts)
                .filter(|(e, _)| e.tag.eq_ignore_ascii_case(&old.tag))
                .filter(|(_, t)| {
                    text.len().min(t.len()) as f64 / text.len().max(t.len()).max(1) as f64 >= 0.8
                })
                .map(|(e, t)| (e.i, similarity(&text, t)))
                .collect();
            scores.sort_by(|a, b| b.1.total_cmp(&a.1));
            let &(index, score) = scores.first()?;
            if score < 0.8 || scores.get(1).is_some_and(|other| score - other.1 < 0.08) {
                None
            } else {
                Some(index)
            }
        })
        .collect();
    let mut counts = std::collections::HashMap::new();
    for index in proposed.iter().flatten() {
        *counts.entry(*index).or_insert(0usize) += 1;
    }
    proposed
        .into_iter()
        .map(|target| target.filter(|i| counts[i] == 1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn element(i: usize, text: &str) -> Element {
        Element {
            i,
            text: text.into(),
            tag: "P".into(),
        }
    }

    #[test]
    fn repeated_source_text_must_not_collapse_onto_one_remaining_target() {
        assert_eq!(
            targets(
                &[element(0, "Repeated"), element(1, "Repeated")],
                &[element(5, "Repeated")]
            ),
            vec![None, None]
        );
    }

    #[test]
    fn repeated_text_can_follow_a_pure_layout_reflow() {
        assert_eq!(
            targets(
                &[element(0, "Repeated"), element(1, "Repeated")],
                &[element(5, "Repeated"), element(6, "Repeated")]
            ),
            vec![Some(5), Some(6)]
        );
    }
}
