//! Semantic matching is server-owned; the renderer supplies actual layout elements.
//! Never trust positional section IDs as identity, or guess between equal matches.
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

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
    targets_within(previous, current, None).expect("without a deadline the comparison finishes")
}

/// The same matching, bounded by `deadline`.
///
/// The comparison is quadratic -- a page's element map can hold a thousand entries --
/// and a request that gives up on it must actually stop doing the work, not merely stop
/// waiting for it. `None` means the deadline passed before every note was placed, and the
/// caller should tell the reader to keep the notes where they are.
pub fn targets_within(
    previous: &[Element],
    current: &[Element],
    deadline: Option<Instant>,
) -> Option<Vec<Option<usize>>> {
    let expired = || deadline.is_some_and(|deadline| Instant::now() >= deadline);

    // A pure reflow/font-size change has no semantic ambiguity, even for repeated
    // labels. Require the entire sequence to match before trusting order.
    if previous.len() == current.len()
        && previous.iter().zip(current).all(|(a, b)| {
            a.tag.eq_ignore_ascii_case(&b.tag) && normalized(&a.text) == normalized(&b.text)
        })
    {
        return Some(current.iter().map(|e| Some(e.i)).collect());
    }
    let texts: Vec<_> = current.iter().map(|e| normalized(&e.text)).collect();

    // Exact text matches are resolved first, so a fuzzy guess can never veto the note of
    // a paragraph that is still there word for word. A deleted paragraph similar to a
    // surviving one used to steal the survivor's match and take its note down with it.
    let mut resolved: Vec<Option<usize>> = vec![None; previous.len()];
    let mut exact_claims: HashMap<usize, usize> = HashMap::new();
    for (slot, old) in previous.iter().enumerate() {
        if expired() {
            return None;
        }
        let text = normalized(&old.text);
        if text.is_empty() {
            continue;
        }
        let exact: Vec<usize> = current
            .iter()
            .zip(&texts)
            .filter(|(e, t)| e.tag.eq_ignore_ascii_case(&old.tag) && **t == text)
            .map(|(e, _)| e.i)
            .collect();
        if exact.len() == 1 {
            resolved[slot] = Some(exact[0]);
            *exact_claims.entry(exact[0]).or_insert(0) += 1;
        }
    }
    // Repeated source text must not collapse onto one remaining target.
    for slot in resolved.iter_mut() {
        if slot.is_some_and(|index| exact_claims[&index] != 1) {
            *slot = None;
        }
    }
    let claimed: HashSet<usize> = resolved.iter().flatten().copied().collect();

    let mut guesses: Vec<Option<usize>> = vec![None; previous.len()];
    let mut guess_claims: HashMap<usize, usize> = HashMap::new();
    for (slot, old) in previous.iter().enumerate() {
        if expired() {
            return None;
        }
        if resolved[slot].is_some() {
            continue;
        }
        let text = normalized(&old.text);
        if text.is_empty() {
            continue;
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
        let Some(&(index, score)) = scores.first() else {
            continue;
        };
        // Never guess between two evenly good candidates, and never take a paragraph an
        // exact match already owns.
        if score < 0.8
            || scores.get(1).is_some_and(|other| score - other.1 < 0.08)
            || claimed.contains(&index)
        {
            continue;
        }
        guesses[slot] = Some(index);
        *guess_claims.entry(index).or_insert(0) += 1;
    }

    resolved.iter_mut().zip(guesses).for_each(|(slot, guess)| {
        if slot.is_none() {
            *slot = guess.filter(|index| guess_claims[index] == 1);
        }
    });
    Some(resolved)
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

    // The rules below are the ones a reader feels. A note that follows the wrong
    // paragraph is worse than one that is left unplaced: the author is told the wrong
    // thing with confidence, and nothing on the page looks wrong.

    fn para(i: usize, text: &str) -> Element {
        Element {
            i,
            text: text.into(),
            tag: "P".into(),
        }
    }

    #[test]
    fn an_inserted_paragraph_does_not_shift_the_notes_below_it() {
        assert_eq!(
            targets(
                &[
                    para(0, "Alpha introduces the problem"),
                    para(1, "Beta explains the fix")
                ],
                &[
                    para(0, "A new opening paragraph"),
                    para(1, "Alpha introduces the problem"),
                    para(2, "Beta explains the fix"),
                ]
            ),
            vec![Some(1), Some(2)]
        );
    }

    #[test]
    fn a_deleted_paragraph_leaves_only_its_own_note_unplaced() {
        assert_eq!(
            targets(
                &[
                    para(0, "Alpha introduces the problem"),
                    para(1, "Beta is being removed"),
                    para(2, "Gamma explains the fix"),
                ],
                &[
                    para(0, "Alpha introduces the problem"),
                    para(1, "Gamma explains the fix")
                ]
            ),
            vec![Some(0), None, Some(1)]
        );
    }

    #[test]
    fn reordered_paragraphs_take_their_notes_with_them() {
        // Position is not identity: the same text in a new place still owns its note.
        assert_eq!(
            targets(
                &[
                    para(0, "Alpha introduces the problem"),
                    para(1, "Beta explains the fix")
                ],
                &[
                    para(0, "Beta explains the fix"),
                    para(1, "Alpha introduces the problem")
                ]
            ),
            vec![Some(1), Some(0)]
        );
    }

    #[test]
    fn a_small_edit_keeps_the_note() {
        assert_eq!(
            targets(
                &[para(
                    0,
                    "The retry loop waits thirty seconds before it gives up"
                )],
                &[para(
                    7,
                    "The retry loop waits sixty seconds before it gives up"
                )]
            ),
            vec![Some(7)]
        );
    }

    #[test]
    fn a_near_tie_is_left_unplaced_rather_than_guessed() {
        assert_eq!(
            targets(
                &[para(0, "abcdefghij")],
                &[para(0, "abcdefghix"), para(1, "abcdefghiy")]
            ),
            vec![None]
        );
    }

    #[test]
    fn empty_text_and_a_changed_tag_are_left_unplaced() {
        assert_eq!(targets(&[para(0, "")], &[para(0, "Anything")]), vec![None]);
        assert_eq!(
            targets(
                &[Element {
                    i: 0,
                    tag: "P".into(),
                    text: "Same words".into()
                }],
                &[Element {
                    i: 0,
                    tag: "H1".into(),
                    text: "Same words".into()
                }]
            ),
            vec![None]
        );
    }

    #[test]
    fn a_short_label_does_not_claim_a_long_paragraph() {
        // "TODOs" is a prefix-length fragment of the paragraph, not its identity; matching
        // it would have moved the note to whatever happened to be the same length.
        assert_eq!(
            targets(
                &[para(0, "TODOs")],
                &[para(
                    0,
                    "TODOs are tracked in the issue tracker for this repository so that nobody forgets them"
                )]
            ),
            vec![None]
        );
    }

    /// Deterministic pseudo-random source; the suite has no random dependency.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self, bound: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as usize) % bound
        }
    }

    #[test]
    fn a_relocated_note_never_lands_on_another_paragraph() {
        // The property the whole module exists for, over many document shapes: either a
        // note follows its own paragraph or it is left unplaced. Never a neighbour.
        let mut rng = Lcg(0x5eed);
        let words = ["alpha", "beta", "gamma", "delta"];
        for round in 0..200 {
            let count = 4 + rng.next(6);
            let previous: Vec<Element> = (0..count)
                .map(|n| {
                    para(
                        n,
                        &format!(
                            "Paragraph {n} covers the {} case in detail",
                            words[rng.next(words.len())]
                        ),
                    )
                })
                .collect();

            let mut current: Vec<Element> = Vec::new();
            for old in &previous {
                if rng.next(4) == 0 {
                    continue; // deleted
                }
                current.push(para(current.len(), &old.text));
                if rng.next(4) == 0 {
                    current.push(para(
                        current.len(),
                        &format!("An inserted paragraph {}", rng.next(1000)),
                    ));
                }
            }

            let found = targets(&previous, &current);
            for (i, target) in found.iter().enumerate() {
                let Some(j) = target else { continue };
                assert_eq!(
                    current[*j].text, previous[i].text,
                    "round {round}: the note for paragraph {i} landed on another paragraph"
                );
            }
            for (i, old) in previous.iter().enumerate() {
                let survivors = current.iter().filter(|c| c.text == old.text).count();
                if survivors == 1 {
                    let j = current.iter().position(|c| c.text == old.text).unwrap();
                    assert_eq!(
                        found[i],
                        Some(j),
                        "round {round}: the note for paragraph {i} was dropped even though \
                         its paragraph survived unchanged"
                    );
                }
            }
        }
    }

    #[test]
    fn a_comparison_past_its_deadline_stops_and_says_it_did_not_finish() {
        // The caller answers the page with "keep them where they are" as soon as this
        // returns None; the point is that the work stops rather than running on in the
        // blocking pool until the whole map has been compared.
        let previous: Vec<Element> = (0..200)
            .map(|i| para(i, &format!("paragraph {i} {}", "x".repeat(90))))
            .collect();
        let current: Vec<Element> = (0..200)
            .map(|i| para(i, &format!("paragraph {i} {}", "y".repeat(90))))
            .collect();

        let started = Instant::now();
        let result = targets_within(
            &previous,
            &current,
            Some(Instant::now() + std::time::Duration::from_millis(20)),
        );
        assert_eq!(
            result, None,
            "an unfinished comparison must not look like a result"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "the comparison ran to the end instead of stopping: {:?}",
            started.elapsed()
        );
    }
}
