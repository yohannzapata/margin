//! Meaning-preservation checks for model output, applied before anything is shown.
//!
//! Words are aligned (LCS over normalized words: lowercase, letters/digits only,
//! so "dont" == "don't" and punctuation/case fixes are free). Every original
//! word the model dropped must be explained: a commonly confused word, a word
//! the rule checker flagged, or a misspelling of a word that took its place.

use crate::inference::{Mode, Style};

/// Words that are legitimately swapped *for each other* when fixing grammar.
/// A dropped word is only explained if its replacement is in the same group.
const GROUPS: &[&[&str]] = &[
    &["there", "their", "theyre"], &["your", "youre"], &["then", "than"], &["to", "too", "two"],
    &["effect", "affect", "effected", "affected"], &["of", "have", "ve", "wouldve", "couldve", "shouldve"],
    &["were", "where", "wear"], &["whose", "whos"], &["lose", "loose"], &["weather", "whether"],
    &["accept", "except"], &["quiet", "quite"], &["a", "an", "the"],
    &["is", "are", "was", "were", "am", "be", "been", "being", "s"], &["has", "have", "had"],
    &["do", "does", "did", "dont", "doesnt", "didnt"], &["go", "goes", "went", "going"],
    &["sent", "send", "sends"], &["want", "wants", "wanted"], &["me", "i", "my"], &["him", "he"],
    &["her", "she"], &["them", "they"], &["us", "we"], &["this", "these"], &["that", "those"],
    &["cancel", "cancelled", "canceled"], &["its", "it", "itis"],
];

fn same_group(a: &str, b: &str) -> bool {
    GROUPS.iter().any(|g| g.contains(&a) && g.contains(&b))
}
const STOP: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "so", "to", "of", "in", "on", "at", "for", "with", "is", "are",
    "was", "were", "be", "it", "its", "i", "you", "we", "they", "he", "she", "that", "this", "my", "your",
];

pub fn norm(w: &str) -> String {
    w.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(norm).filter(|w| !w.is_empty()).collect()
}

fn lev(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + (a[i - 1] != b[j - 1]) as usize);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Indices of original / candidate words that are *not* part of the alignment.
fn unaligned(a: &[String], b: &[String]) -> (Vec<usize>, Vec<usize>) {
    let (m, n) = (a.len(), b.len());
    let mut dp = vec![vec![0u16; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i][j] = if a[i] == b[j] { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut ra, mut rb) = (0, 0, vec![], vec![]);
    while i < m || j < n {
        if i < m && j < n && a[i] == b[j] {
            i += 1;
            j += 1;
        } else if j < n && (i == m || dp[i][j + 1] >= dp[i + 1][j]) {
            rb.push(j);
            j += 1;
        } else {
            ra.push(i);
            i += 1;
        }
    }
    (ra, rb)
}

/// Tokens that must survive any mode: numbers, mentions, links, emoji, and
/// mid-sentence capitalized words (names).
fn protected(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut sentence_start = true;
    for raw in s.split_whitespace() {
        let w = norm(raw);
        let special = raw.chars().any(|c| c.is_ascii_digit() || c == '@' || c == '#' || (c as u32) > 0x2600) || raw.contains("http");
        let pronoun_i = w == "i" || raw.starts_with("I'") || raw.starts_with("I’");
        let name = !sentence_start && raw.chars().next().is_some_and(char::is_uppercase) && !pronoun_i && w.len() > 1;
        if (special || name) && !w.is_empty() {
            out.push(w);
        }
        sentence_start = raw.ends_with(['.', '!', '?', ':']);
    }
    out
}

/// Casual short forms that must survive every mode (casual stays casual).
const CASUAL: &[&str] = &[
    "u", "ur", "rn", "lol", "lmao", "gonna", "wanna", "gotta", "idk", "tbh", "btw", "omg", "imo", "pls", "plz",
    "thx", "ty", "ya", "yall", "ok", "kinda", "sorta", "nvm", "brb", "jk",
];

/// Words that flip or define meaning. If the original has one, the revision must too.
const NEGATION: &[&str] = &[
    "not", "no", "never", "nothing", "none", "nobody", "nowhere", "neither", "nor", "without", "cant", "cannot",
    "dont", "doesnt", "didnt", "wont", "wouldnt", "shouldnt", "couldnt", "isnt", "arent", "wasnt", "werent",
    "havent", "hasnt", "hadnt", "aint",
];
const THANKS: &[&str] = &["thanks", "thank", "thx", "ty", "grateful", "appreciate", "appreciated"];
const SORRY: &[&str] = &["sorry", "apologies", "apologize", "apologise", "apology"];

fn has_any(w: &[String], set: &[&str]) -> bool {
    w.iter().any(|x| set.contains(&x.as_str()))
}

/// What the rule checker found, for validating model edits.
pub struct Rules<'a> {
    /// (original, suggestion) pairs.
    pub flagged: &'a [(String, String)],
    /// Words judged to be from another language: must be kept.
    pub foreign: &'a [String],
    /// The rule checker ran. When it did, a dictionary word it didn't flag is
    /// spelled correctly and may not be "respelled" (e.g. sarahs → saras).
    pub ran: bool,
    /// Rephrase style: formal/academic may write out slang; creative may reword more.
    pub style: Style,
}

#[derive(Debug, PartialEq)]
pub enum Reject {
    Empty,
    Commentary,
    Unchanged,
    LostToken(String),
    Reworded(String),
    TooDifferent,
    InventedEnding,
    LineBreaks,
    MeaningFlip(&'static str),
}

/// `check_with` for when the rule checker ran and found `flagged`.
pub fn check(mode: Mode, orig: &str, cand: &str, finished: bool, flagged: &[(String, String)]) -> Result<String, Reject> {
    check_with(mode, orig, cand, finished, &Rules { flagged, foreign: &[], ran: true, style: Style::Standard })
}

/// Validate (and lightly repair) a model revision of `orig`.
pub fn check_with(mode: Mode, orig: &str, cand: &str, finished: bool, rules: &Rules) -> Result<String, Reject> {
    let mut c = cand.trim().to_string();
    if c.is_empty() {
        return Err(Reject::Empty);
    }
    let lower = c.to_lowercase();
    if ["here is", "here's the", "sure", "corrected", "rewritten", "certainly", "note:"].iter().any(|p| lower.starts_with(p))
        || c.contains("**")
        || c.contains("```")
    {
        return Err(Reject::Commentary);
    }
    if c.contains('\n') && !orig.contains('\n') {
        return Err(Reject::LineBreaks);
    }
    let (a, b) = (words(orig), words(&c));
    // Casual short forms stay, except when the user asked for a different register.
    let keep_casual = mode != Mode::Rephrase || rules.style == Style::Standard;
    let casual = a.iter().filter(|w| keep_casual && CASUAL.contains(&w.as_str())).cloned();
    let foreign = rules.foreign.iter().map(|w| norm(w));
    for t in protected(orig).into_iter().chain(casual).chain(foreign) {
        if !b.contains(&t) {
            return Err(Reject::LostToken(t));
        }
    }
    if has_any(&a, NEGATION) != has_any(&b, NEGATION) {
        return Err(Reject::MeaningFlip("negation"));
    }
    if has_any(&a, THANKS) && !has_any(&b, THANKS) {
        return Err(Reject::MeaningFlip("thanks"));
    }
    if has_any(&a, SORRY) && !has_any(&b, SORRY) {
        return Err(Reject::MeaningFlip("apology"));
    }

    // An unfinished sentence must end where the user stopped typing.
    if !finished {
        let (la, lb) = (a.last().cloned().unwrap_or_default(), b.last().cloned().unwrap_or_default());
        let same_end = la == lb || same_group(&la, &lb) || (la.len() >= 3 && lev(&la, &lb) <= 2);
        if !same_end {
            return Err(Reject::InventedEnding);
        }
        if !orig.trim_end().ends_with(['.', '!', '?']) {
            c = c.trim_end_matches(['.', '!', '?']).to_string();
        }
    }

    let (ra, rb) = unaligned(&a, &b);
    let n = a.len().max(1);
    match mode {
        Mode::Correct => {
            for &i in &ra {
                let w = &a[i];
                let confusable = rb.iter().any(|&j| same_group(w, &b[j])) || ["a", "an", "the"].contains(&w.as_str());
                // A flagged word may only be replaced by what the checker suggested (or removed if it said so).
                let hinted = rules.flagged.iter().any(|(o, r)| norm(o) == *w && (words(r).is_empty() || words(r).iter().all(|rw| b.contains(rw))));
                // Without rule-checker information, allow a near-identical respelling.
                let misspelling = !rules.ran && w.len() >= 3 && rb.iter().any(|&j| lev(w, &b[j]) <= if w.len() <= 4 { 1 } else { 2 });
                let doubled = i > 0 && a[i - 1] == *w;
                if !(confusable || hinted || misspelling || doubled) {
                    return Err(Reject::Reworded(w.clone()));
                }
            }
            // Additions beyond replacements: a conjunction or "have" is fine, a new clause is not.
            let extra = rb.len().saturating_sub(ra.len());
            if extra > (n / 8).max(1) {
                return Err(Reject::TooDifferent);
            }
        }
        Mode::Improve => {
            if ra.len() * 100 / n > 45 || rb.len().saturating_sub(ra.len()) * 100 / n > 40 {
                return Err(Reject::TooDifferent);
            }
        }
        Mode::Rephrase => {
            let content: Vec<&String> = a.iter().filter(|w| w.len() >= 4 && !STOP.contains(&w.as_str())).collect();
            let kept = content.iter().filter(|w| b.contains(w)).count();
            let ratio = b.len() as f32 / n as f32;
            // How much wording must survive, and how much longer/shorter it may get, per style.
            let (min_kept, len_range) = match rules.style {
                Style::Standard => (20, 0.5..=1.8),
                Style::Simple => (15, 0.4..=1.6),
                Style::Formal => (15, 0.5..=2.2),
                // Measured: correct academic/creative paraphrases replace most words.
                Style::Academic | Style::Creative => (10, 0.5..=2.2),
            };
            if (!content.is_empty() && kept * 100 / content.len() < min_kept) || !len_range.contains(&ratio) {
                return Err(Reject::TooDifferent);
            }
        }
    }
    // Punctuation-only edits are real edits; only an identical text is "unchanged".
    if mode != Mode::Correct && c.trim() == orig.trim() {
        return Err(Reject::Unchanged);
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Mode::*;

    #[test]
    fn accepts_real_corrections() {
        let o = "hey i was thinking about what you said yesterday honestly i dont think thats going to work";
        let c = "Hey, I was thinking about what you said yesterday. Honestly, I don't think that's going to work.";
        assert_eq!(check(Correct, o, c, true, &[]), Ok(c.into()));
        assert!(check(Correct, "there going to be here soon, put the bags over their", "They're going to be here soon. Put the bags over there.", true, &[]).is_ok());
        assert!(check(Correct, "I would of helped", "I would have helped.", true, &[]).is_ok());
        let f = vec![("tomorow".to_string(), "tomorrow".to_string())];
        assert!(check(Correct, "see you tomorow", "See you tomorrow.", true, &f).is_ok());
    }

    #[test]
    fn rejects_meaning_changes_seen_in_baseline() {
        // All four were accepted by the v0.1 length check.
        assert!(check(Correct, "there place is huge and its close to the station", "There's a place that's huge and close to the station", true, &[]).is_err());
        assert!(check(Correct, "ok sige, see you later na lang kasi busy pa ako", "ok, see you later na lang, kasi busy pa ako", true, &[]).is_err());
        assert!(check(Correct, "cant wait to see u guys", "can't wait to see you guys", true, &[]).is_err());
        assert!(check(Correct, "The dog wagged its tail when they're owner came home.", "The dog wagged its tail when the owner came home.", true, &[]).is_err());
        assert_eq!(check(Correct, "thanks John I appreciate it", "Thanks, I appreciate it.", true, &[]), Err(Reject::LostToken("john".into())));
    }

    #[test]
    fn unfinished_sentences_keep_their_end() {
        let o = "I dont think thats a good idea because its";
        assert_eq!(check(Correct, o, "I don't think that's a good idea because it's.", false, &[]).as_deref(), Ok("I don't think that's a good idea because it's"));
        assert_eq!(check(Correct, o, "I don't think that's a good idea because it seems risky.", false, &[]), Err(Reject::InventedEnding));
        // A finished thought may gain its final period.
        assert_eq!(check(Correct, "thats pretty good", "That's pretty good.", true, &[]).as_deref(), Ok("That's pretty good."));
    }

    #[test]
    fn flagged_words_must_take_the_suggestion() {
        let f = vec![("rn".to_string(), "RN".to_string())];
        // v0.1/1.5B dropped "rn" and added "I'm": not what the checker suggested.
        assert!(check(Correct, "gonna be late, traffic is crazy rn", "I'm gonna be late. Traffic is crazy.", true, &f).is_err());
        let f = vec![("dont".to_string(), "don't".to_string())];
        assert!(check(Correct, "i dont know", "I don't know.", true, &f).is_ok());
    }

    #[test]
    fn meaning_guards_from_reading_4b_output() {
        // Each candidate below was produced by Qwen3-4B on the suite and read as wrong.
        let thanks = "Thanks so much, I'll see you on Monday!";
        assert_eq!(check(Rephrase, thanks, "You're very welcome, I'll catch up with you on Monday!", true, &[]), Err(Reject::MeaningFlip("thanks")));
        assert_eq!(check(Improve, "cant wait to see u guys", "Can't wait to see you guys.", true, &[]), Err(Reject::LostToken("u".into())));
        assert_eq!(check(Improve, "traffic is crazy rn", "Traffic is crazy right now.", true, &[]), Err(Reject::LostToken("rn".into())));
        let foreign = vec!["sige".to_string()];
        let r = Rules { flagged: &[], foreign: &foreign, ran: true, style: Style::Standard };
        assert!(check_with(Rephrase, "ok sige, see you later", "Alright, see you later.", true, &r).is_err());
        // A correctly spelled (unflagged) name may not be respelled.
        assert_eq!(check(Correct, "we're at sarahs place", "We're at Sara's place.", true, &[]), Err(Reject::Reworded("sarahs".into())));
        assert!(check(Improve, "I don't like it", "I like it.", true, &[]).is_err());
    }

    #[test]
    fn formal_may_write_out_slang_standard_may_not() {
        let o = "gonna be late, traffic is crazy rn";
        let c = "I am going to be late; the traffic is very heavy right now.";
        let std = Rules { flagged: &[], foreign: &[], ran: true, style: Style::Standard };
        let formal = Rules { flagged: &[], foreign: &[], ran: true, style: Style::Formal };
        assert!(check_with(Rephrase, o, c, true, &std).is_err());
        assert!(check_with(Rephrase, o, c, true, &formal).is_ok());
        // Negation still can't be lost in any style.
        assert!(check_with(Rephrase, "I can't come tonight.", "I will come tonight.", true, &formal).is_err());
    }

    #[test]
    fn punctuation_only_improvement_is_not_unchanged() {
        assert!(check(Improve, "what do you think about it", "What do you think about it?", true, &[]).is_ok());
    }

    #[test]
    fn commentary_is_rejected() {
        assert_eq!(check(Correct, "hi", "Here is the corrected text: Hi.", true, &[]), Err(Reject::Commentary));
    }

    #[test]
    fn rephrase_must_keep_some_substance() {
        let o = "I don't think that's a good idea because it's too expensive.";
        assert!(check(Rephrase, o, "I'm not sure that's a good idea, since it costs too much.", true, &[]).is_ok());
        assert!(check(Rephrase, o, "Let's go to the beach tomorrow.", true, &[]).is_err());
        assert_eq!(check(Rephrase, o, o, true, &[]), Err(Reject::Unchanged));
    }
}
