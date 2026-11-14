//! Tier 1: instant, offline spelling/grammar/punctuation via Harper (Apache-2.0).
//! Runs in ~2-5 ms per paragraph. Its findings are shown immediately and are
//! also passed to the model as hints.

use harper_core::linting::{LintGroup, LintKind, Linter, Suggestion};
use harper_core::parsers::PlainEnglish;
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};

pub struct Grammar {
    linter: LintGroup,
    dialect: Dialect,
}

#[derive(Debug, Default)]
pub struct Analysis {
    /// The text with every confident fix applied, if anything changed.
    pub fixed: Option<String>,
    /// Findings for the model, e.g. "dont → don't".
    pub hints: Vec<String>,
    /// (original, suggestion) pairs, used to validate model edits.
    pub flagged: Vec<(String, String)>,
    /// Several "misspellings" that aren't apostrophe slips: likely another language.
    pub mixed_language: bool,
    /// Those words, which every revision must keep.
    pub foreign: Vec<String>,
}

fn letters(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

impl Grammar {
    pub fn new(dialect: Dialect) -> Self {
        Grammar { linter: LintGroup::new_curated(FstDictionary::curated(), dialect), dialect }
    }

    pub fn set_dialect(&mut self, dialect: Dialect) {
        if dialect != self.dialect {
            *self = Grammar::new(dialect);
        }
    }

    /// Convenience for callers that only want the fixed text.
    pub fn correct(&mut self, text: &str, finished: bool) -> Option<String> {
        self.analyze(text, finished).fixed
    }

    /// `finished` = the user has ended the sentence; otherwise the last (possibly
    /// half-typed) word is left alone so we don't "fix" words still being typed.
    pub fn analyze(&mut self, text: &str, finished: bool) -> Analysis {
        let doc = Document::new_curated(text, &PlainEnglish);
        let src: Vec<char> = text.chars().collect();
        let mut lints = self.linter.lint(&doc);

        let last_word_start = if finished || text.ends_with(char::is_whitespace) {
            src.len()
        } else {
            src.iter().rposition(|c| c.is_whitespace()).map(|i| i + 1).unwrap_or(0)
        };
        lints.retain(|l| l.span.end <= last_word_start && !l.suggestions.is_empty());
        lints.retain(|l| !(l.lint_kind == LintKind::Capitalization && l.span.start == 0 && !finished && src.len() < 12));

        let orig_of = |l: &harper_core::linting::Lint| -> String { src[l.span.start..l.span.end].iter().collect() };
        let repl_of = |s: &Suggestion, orig: &str| -> String {
            match s {
                Suggestion::ReplaceWith(r) => r.iter().collect(),
                Suggestion::InsertAfter(r) => format!("{orig}{}", r.iter().collect::<String>()),
                Suggestion::Remove => String::new(),
            }
        };

        // Prefer a suggestion that only adds an apostrophe or capital letter over a
        // different word: "sarahs" → "Sarah's", not "Sara's".
        let words = |s: &str| s.split_whitespace().count();
        let best = |l: &harper_core::linting::Lint| -> String {
            let o = orig_of(l);
            let all: Vec<String> = l.suggestions.iter().map(|s| repl_of(s, &o)).collect();
            if let Some(r) = all.iter().find(|r| l.lint_kind == LintKind::Spelling && letters(r) == letters(&o)) {
                return r.clone();
            }
            // Prefer a suggestion that keeps every word ("its the" → "it's the", not "its").
            all.iter().find(|r| words(r) >= words(&o)).cloned().unwrap_or_else(|| all[0].clone())
        };
        // A fix that deletes words is only trusted when the flagged text really repeats a
        // word ("the the"). Measured: Harper turns "its the best" into "its best".
        let repeats = |s: &str| {
            let w: Vec<String> = s.split_whitespace().map(letters).collect();
            w.windows(2).any(|p| p[0] == p[1])
        };
        let drops_words = |l: &harper_core::linting::Lint| words(&best(l)) < words(&orig_of(l)) && !repeats(&orig_of(l));
        lints.retain(|l| !drops_words(l));

        // A spelling flag whose fix only adds an apostrophe ("dont" → "don't") is an
        // English slip. Several flags that aren't suggest words from another language.
        let foreign: Vec<bool> = lints
            .iter()
            .map(|l| l.lint_kind == LintKind::Spelling && letters(&best(l)) != letters(&orig_of(l)))
            .collect();
        let words = text.split_whitespace().count().max(1);
        let n_foreign = foreign.iter().filter(|f| **f).count();
        let mixed_language = n_foreign >= 2 && n_foreign * 100 / words >= 15;

        let mut a = Analysis { mixed_language, ..Default::default() };
        let mut kept: Vec<&harper_core::linting::Lint> = Vec::new();
        for (l, f) in lints.iter().zip(&foreign) {
            if mixed_language && *f {
                a.foreign.push(orig_of(l)); // leave non-English words alone
                continue;
            }
            let o = orig_of(l);
            let r = best(l);
            let hint = format!("{o} → {r}");
            if !a.hints.contains(&hint) {
                a.hints.push(hint);
            }
            a.flagged.push((o, r));
            kept.push(l);
        }

        kept.sort_by_key(|l| (l.span.start, l.priority));
        let mut out: Vec<char> = Vec::with_capacity(src.len() + 8);
        let mut pos = 0;
        let mut changed = false;
        for l in kept {
            if l.span.start < pos {
                continue;
            }
            out.extend_from_slice(&src[pos..l.span.start]);
            out.extend(best(l).chars());
            pos = l.span.end;
            changed = true;
        }
        if changed {
            out.extend_from_slice(&src[pos..]);
            let fixed: String = out.into_iter().collect();
            let fixed = fixed.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ") + if text.ends_with(' ') { " " } else { "" };
            if fixed != text {
                a.fixed = Some(fixed);
            }
        }
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_contractions_but_not_word_in_progress() {
        let mut g = Grammar::new(Dialect::American);
        let out = g.correct("I dont think thats a good idea because its", false).unwrap();
        assert!(out.starts_with("I don't think that's a good idea"), "{out}");
        assert!(out.ends_with("because its"), "{out}");
    }

    #[test]
    fn fixes_finished_sentence() {
        let mut g = Grammar::new(Dialect::American);
        assert_eq!(g.correct("This is an test of the the system. ", true).unwrap(), "This is a test of the system. ");
    }

    #[test]
    fn clean_sentence_is_none() {
        let mut g = Grammar::new(Dialect::American);
        assert_eq!(g.correct("The meeting is at three tomorrow.", true), None);
    }

    #[test]
    fn does_not_drop_words_from_correct_text() {
        let mut g = Grammar::new(Dialect::American);
        let a = g.analyze("we went to the beach yesterday and i think its the best day of the summer", true);
        let fixed = a.fixed.unwrap_or_default();
        assert!(fixed.contains("the best day"), "{fixed:?} flagged {:?}", a.flagged);
    }

    #[test]
    fn hints_for_the_model() {
        let mut g = Grammar::new(Dialect::American);
        let a = g.analyze("i dont know", true);
        assert!(a.hints.iter().any(|h| h == "dont → don't"), "{:?}", a.hints);
        assert!(!a.mixed_language);
    }

    #[test]
    fn apostrophe_fix_preferred_over_other_word() {
        let mut g = Grammar::new(Dialect::American);
        let a = g.analyze("we are at sarahs place", true);
        assert!(a.flagged.iter().any(|(o, r)| o == "sarahs" && r == "Sarah's"), "{:?}", a.flagged);
    }

    #[test]
    fn mixed_language_words_are_left_alone() {
        let mut g = Grammar::new(Dialect::American);
        let a = g.analyze("ok sige, see you later na lang kasi busy pa ako", true);
        assert!(a.mixed_language, "{:?}", a.hints);
        let fixed = a.fixed.unwrap_or_default();
        assert!(!fixed.contains("side") && !a.hints.iter().any(|h| h.starts_with("sige")), "{fixed} {:?}", a.hints);
    }
}
