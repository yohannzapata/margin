//! Pure text utilities: sentence boundaries, word diffs, edit planning.
//! Everything here is platform-independent and unit-tested.

/// The editable region the assistant works on: the sentence that ends at the caret.
#[derive(Debug, Clone, PartialEq)]
pub struct Sentence {
    /// Sentence text (no leading whitespace). May include trailing whitespace
    /// when the user has just finished a sentence ("Hi there. |").
    pub text: String,
    /// True when the sentence ends with terminal punctuation (+ optional spaces).
    pub finished: bool,
}

const TERMINATORS: [char; 3] = ['.', '!', '?'];

/// Extract the sentence the caret is in (or the one just finished).
/// `before` is the text from the start of the field up to the caret.
pub fn current_sentence(before: &str) -> Sentence {
    // Never cross a line break: chat messages and emails are line-oriented.
    let line = before.rsplit(['\n', '\r']).next().unwrap_or("");
    let chars: Vec<char> = line.chars().collect();

    // Skip trailing whitespace and terminators to find where "this" sentence ends.
    let mut end = chars.len();
    while end > 0 && chars[end - 1].is_whitespace() {
        end -= 1;
    }
    let finished = end > 0 && TERMINATORS.contains(&chars[end - 1]);
    let mut scan = end;
    while scan > 0 && TERMINATORS.contains(&chars[scan - 1]) {
        scan -= 1;
    }
    // Walk back to the previous terminator followed by whitespace.
    let mut start = 0;
    let mut i = scan;
    while i > 1 {
        if chars[i - 1].is_whitespace() && TERMINATORS.contains(&chars[i - 2]) {
            start = i;
            break;
        }
        i -= 1;
    }
    while start < chars.len() && chars[start].is_whitespace() {
        start += 1;
    }
    Sentence { text: chars[start..].iter().collect(), finished }
}

/// The message being composed, split around the paragraph the caret is in.
/// The model reads all of it; edits are applied to `target` only, so a
/// suggestion never has to retype a line break (Enter sends chat messages).
#[derive(Debug, Clone, PartialEq)]
pub struct Composition {
    /// Earlier paragraphs (read-only context), up to ~800 chars.
    pub before: String,
    /// The current paragraph from its first non-space character to the caret.
    pub target: String,
    /// Text after the caret (read-only context), up to ~400 chars.
    pub after: String,
    /// The paragraph ends with sentence punctuation (optionally followed by spaces).
    pub ends_sentence: bool,
}

pub fn composition(field_before: &str, field_after: &str) -> Composition {
    let cut = field_before.rfind(['\n', '\r']).map(|i| i + 1).unwrap_or(0);
    let para = &field_before[cut..];
    let lead = para.len() - para.trim_start().len();
    let target = para[lead..].to_string();
    let before_all = &field_before[..cut + lead];
    let before: String = {
        let c: Vec<char> = before_all.chars().collect();
        c[c.len().saturating_sub(800)..].iter().collect()
    };
    let after: String = field_after.chars().take(400).collect();
    let ends_sentence = target.trim_end().ends_with(TERMINATORS);
    Composition { before, target, after, ends_sentence }
}

/// Number of words in a string.
pub fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Approximate the number of caret positions (grapheme clusters) in `s`, which is
/// how many Shift+Left presses select it. Handles combining marks, variation
/// selectors, skin-tone modifiers and ZWJ emoji sequences.
pub fn caret_stops(s: &str) -> usize {
    let mut n = 0;
    let mut joined = false;
    for c in s.chars() {
        let cp = c as u32;
        let extends = matches!(cp, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x20D0..=0x20FF | 0xFE00..=0xFE0F | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F);
        if cp == 0x200D {
            joined = true;
            continue;
        }
        if extends || joined {
            joined = false;
            continue;
        }
        n += 1;
    }
    // "\r\n" is a single caret stop.
    n - s.matches("\r\n").count()
}

/// Minimal edit to turn `old` (text ending at the caret) into `new`:
/// keep the common prefix, select `select` caret stops backwards, type `insert`.
#[derive(Debug, Clone, PartialEq)]
pub struct EditPlan {
    pub select: usize,
    pub insert: String,
}

pub fn plan_edit(old: &str, new: &str) -> EditPlan {
    let o: Vec<char> = old.chars().collect();
    let n: Vec<char> = new.chars().collect();
    let mut p = 0;
    while p < o.len() && p < n.len() && o[p] == n[p] {
        p += 1;
    }
    // Back up to a word start so we never split a grapheme or a word mid-way
    // (only needed when part of the old text actually changes).
    while p < o.len() && p > 0 && !o[p - 1].is_whitespace() {
        p -= 1;
    }
    let removed: String = o[p..].iter().collect();
    EditPlan { select: caret_stops(&removed), insert: n[p..].iter().collect() }
}

/// A run of display text; `changed` runs are highlighted in the popup.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    pub changed: bool,
}

/// Word-level diff of `new` against `old` (LCS). Returns runs of `new`.
pub fn diff_words(old: &str, new: &str) -> Vec<Run> {
    let a: Vec<&str> = old.split_whitespace().collect();
    let b: Vec<&str> = new.split_whitespace().collect();
    let (m, n) = (a.len(), b.len());
    let mut dp = vec![vec![0u16; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i][j] = if a[i] == b[j] { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
        }
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut push = |w: &str, changed: bool| {
        if let Some(last) = runs.last_mut() {
            if last.changed == changed {
                last.text.push(' ');
                last.text.push_str(w);
                return;
            }
            // Separator space belongs to the unchanged side for cleaner highlights.
            if changed {
                last.text.push(' ');
                runs.push(Run { text: w.to_string(), changed });
            } else {
                runs.push(Run { text: format!(" {w}"), changed });
            }
            return;
        }
        runs.push(Run { text: w.to_string(), changed });
    };
    let (mut i, mut j) = (0, 0);
    while j < n {
        if i < m && a[i] == b[j] {
            push(b[j], false);
            i += 1;
            j += 1;
        } else if i < m && dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            push(b[j], true);
            j += 1;
        }
    }
    runs
}

/// One individually fixable mistake: replace chars `start..end` of the paragraph with `repl`.
#[derive(Debug, Clone, PartialEq)]
pub struct Hunk {
    pub start: usize,
    pub end: usize,
    pub orig: String,
    pub repl: String,
    pub category: &'static str,
}

fn tokens(s: &str) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let st = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        out.push((st, i, chars[st..i].iter().collect()));
    }
    out
}

fn letters_lower(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Label for the popup, like QuillBot/Grammarly categories.
pub fn category(orig: &str, repl: &str) -> &'static str {
    let (lo, lr) = (letters_lower(orig), letters_lower(repl));
    if repl.is_empty() {
        return "Remove";
    }
    if lo == lr {
        let strip = |s: &str| s.chars().filter(|c| *c != '\'' && *c != '’').collect::<String>();
        let punct = |s: &str| s.chars().filter(|c| !c.is_alphanumeric()).collect::<String>();
        if strip(orig) == strip(repl) && orig.contains('\'') != repl.contains('\'') || strip(orig).to_lowercase() == strip(repl).to_lowercase() && punct(orig) == punct(repl) && orig != repl && (orig.contains('\'') || repl.contains('\'')) {
            return "Spelling"; // dont → don't
        }
        if punct(orig) == punct(repl) {
            return "Capitalization";
        }
        return "Punctuation";
    }
    let single = !orig.trim().contains(' ') && !repl.trim().contains(' ');
    if single && letters_lower(orig).len() >= 3 {
        let (a, b): (Vec<char>, Vec<char>) = (lo.chars().collect(), lr.chars().collect());
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for i in 1..=a.len() {
            let mut cur = vec![i; b.len() + 1];
            for j in 1..=b.len() {
                cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + (a[i - 1] != b[j - 1]) as usize);
            }
            prev = cur;
        }
        if prev[b.len()] <= 2 {
            return "Spelling";
        }
    }
    "Grammar"
}

/// Split the difference between `orig` and `fixed` into independent fixes.
/// Offsets are char indices into `orig`.
pub fn hunks(orig: &str, fixed: &str) -> Vec<Hunk> {
    let (a, b) = (tokens(orig), tokens(fixed));
    let (m, n) = (a.len(), b.len());
    let mut dp = vec![vec![0u16; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i][j] = if a[i].2 == b[j].2 { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
        }
    }
    // Unmatched runs: (a_from, a_to, b_from, b_to) in token indices.
    let mut runs: Vec<(usize, usize, usize, usize)> = vec![];
    let (mut i, mut j) = (0, 0);
    while i < m || j < n {
        if i < m && j < n && a[i].2 == b[j].2 {
            i += 1;
            j += 1;
            continue;
        }
        let (i0, j0) = (i, j);
        while (i < m || j < n) && !(i < m && j < n && a[i].2 == b[j].2) {
            if j < n && (i == m || dp[i][j + 1] >= dp[i + 1][j]) { j += 1 } else { i += 1 }
        }
        runs.push((i0, i, j0, j));
    }
    let fchars: Vec<char> = fixed.chars().collect();
    let ochars: Vec<char> = orig.chars().collect();
    let btext = |k0: usize, k1: usize| -> String { fchars[b[k0].0..b[k1 - 1].1].iter().collect() };
    let mut out = vec![];
    for (i0, i1, j0, j1) in runs {
        if i1 - i0 == j1 - j0 && i1 > i0 {
            // Same number of words on both sides: one fix per word.
            for k in 0..(i1 - i0) {
                let (s, e, ref o) = a[i0 + k];
                let r = b[j0 + k].2.clone();
                out.push(Hunk { start: s, end: e, orig: o.clone(), repl: r.clone(), category: category(o, &r) });
            }
            continue;
        }
        let (s, e, repl) = if i1 > i0 {
            let repl = if j1 > j0 { btext(j0, j1) } else { String::new() };
            let (s, mut e) = (a[i0].0, a[i1 - 1].1);
            if repl.is_empty() {
                // A removal also takes one neighbouring space.
                while e < ochars.len() && ochars[e].is_whitespace() {
                    e += 1;
                }
            }
            (s, e, repl)
        } else if i0 > 0 && j0 > 0 {
            // Pure insertion: attach to the word before it.
            (a[i0 - 1].0, a[i0 - 1].1, btext(j0 - 1, j1))
        } else if i0 < m && j1 < n {
            (a[i0].0, a[i0].1, btext(j0, j1 + 1))
        } else {
            continue;
        };
        let o: String = ochars[s..e].iter().collect();
        let category = category(o.trim(), &repl);
        out.push(Hunk { start: s, end: e, orig: o, repl, category });
    }
    out.retain(|h| h.orig.trim() != h.repl.trim());
    out
}

/// `orig` with only `h` applied.
pub fn apply_hunk(orig: &str, h: &Hunk) -> String {
    let c: Vec<char> = orig.chars().collect();
    let tail = &c[h.end..];
    let mut s: String = c[..h.start].iter().collect();
    s.push_str(&h.repl);
    s.extend(tail);
    s
}

/// Clean a model's single-line answer: strip quotes, labels and extra lines.
pub fn clean_model_line(s: &str) -> String {
    let mut t = s.trim();
    for prefix in ["Corrected:", "Corrected text:", "Output:", "Rewrite:", "- ", "* "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            t = rest.trim();
        }
    }
    // Leading list numbering like "1. " or "2) ".
    let b = t.as_bytes();
    if b.len() > 3 && b[0].is_ascii_digit() && (b[1] == b'.' || b[1] == b')') && b[2] == b' ' {
        t = &t[3..];
    }
    let t = t.trim_matches(|c| c == '"' || c == '“' || c == '”');
    t.trim().to_string()
}

/// Is a proposed rewrite of `orig` plausible (not a hallucinated essay, not empty)?
pub fn plausible_rewrite(orig: &str, cand: &str) -> bool {
    let (lo, lc) = (orig.trim().chars().count().max(1) as f32, cand.trim().chars().count() as f32);
    !cand.trim().is_empty() && lc / lo > 0.5 && lc / lo < 1.8 && !cand.contains('\n')
}

/// Small models tend to "finish" a sentence the user is still typing. For an
/// unfinished input, drop added final punctuation, and reject outputs that add
/// more than one word (one is allowed for expansions like "its" -> "it is").
pub fn trim_invented_ending(orig: &str, fixed: &str) -> Option<String> {
    let o = orig.trim_end();
    if o.ends_with(TERMINATORS) {
        return Some(fixed.to_string());
    }
    if word_count(fixed) > word_count(o) + 1 {
        return None;
    }
    Some(fixed.trim_end().trim_end_matches(TERMINATORS).to_string())
}

/// Keep the trailing whitespace of `orig` on `fixed` (corrections must not eat the space the user typed).
pub fn keep_trailing_ws(orig: &str, fixed: &str) -> String {
    let ws: String = orig.chars().rev().take_while(|c| c.is_whitespace()).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{}{}", fixed.trim_end(), ws)
}

/// Normalize a predicted continuation for insertion after `before`.
pub fn normalize_completion(before: &str, raw: &str) -> Option<String> {
    let first_line = raw.split(['\n', '\r']).next().unwrap_or("");
    // Limit to a few words: predictions should be glanceable.
    let mut words = 0;
    let mut out = String::new();
    let mut in_word = false;
    for c in first_line.chars() {
        if c.is_whitespace() {
            if in_word {
                words += 1;
                in_word = false;
                if words >= 4 {
                    break;
                }
            }
        } else {
            in_word = true;
        }
        out.push(c);
    }
    let mut out = out.trim_end().to_string();
    if before.ends_with(char::is_whitespace) {
        out = out.trim_start().to_string();
    }
    // Fill-in-the-blank artifacts ("_____ my car") and markup are not predictions.
    if out.contains(['_', '<', '>', '[', ']', '*', '#']) {
        return None;
    }
    if out.chars().any(char::is_alphanumeric) { Some(out) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_in_progress() {
        let s = current_sentence("Hello there. I dont think thats a good idea because its");
        assert_eq!(s.text, "I dont think thats a good idea because its");
        assert!(!s.finished);
    }

    #[test]
    fn sentence_just_finished() {
        let s = current_sentence("First one. their going home. ");
        assert_eq!(s.text, "their going home. ");
        assert!(s.finished);
    }

    #[test]
    fn sentence_stops_at_newline() {
        assert_eq!(current_sentence("Hi Sam,\nim late").text, "im late");
        assert_eq!(current_sentence("Line\r\n").text, "");
    }

    #[test]
    fn sentence_ignores_decimals() {
        assert_eq!(current_sentence("It costs 3.50 dollars").text, "It costs 3.50 dollars");
    }

    #[test]
    fn edit_plan_keeps_prefix() {
        let p = plan_edit("I dont think thats ok", "I don't think that's ok");
        assert_eq!(p, EditPlan { select: "dont think thats ok".len(), insert: "don't think that's ok".into() });
        let p = plan_edit("same", "same");
        assert_eq!(p.select, 0);
        assert_eq!(p.insert, "");
    }

    #[test]
    fn caret_stops_emoji() {
        assert_eq!(caret_stops("ab"), 2);
        assert_eq!(caret_stops("👍🏽"), 1);
        assert_eq!(caret_stops("👨‍👩‍👧"), 1);
        assert_eq!(caret_stops("e\u{301}"), 1);
        assert_eq!(caret_stops("a\r\nb"), 3);
    }

    #[test]
    fn diff_marks_changes() {
        let runs = diff_words("I dont think thats ok", "I don't think that's ok");
        let changed: Vec<_> = runs.iter().filter(|r| r.changed).map(|r| r.text.as_str()).collect();
        assert_eq!(changed, vec!["don't", "that's"]);
        let joined: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(joined, "I don't think that's ok");
    }

    #[test]
    fn completion_normalization() {
        assert_eq!(normalize_completion("Thanks so ", " much for the help today and more"), Some("much for the help".into()));
        assert_eq!(normalize_completion("Thanks so", " much"), Some(" much".into()));
        assert_eq!(normalize_completion("x ", " ..."), None);
    }

    #[test]
    fn cleaning_model_output() {
        assert_eq!(clean_model_line("\"Hello there.\""), "Hello there.");
        assert_eq!(clean_model_line("1. I'm not sure."), "I'm not sure.");
        assert_eq!(clean_model_line("Corrected: It's fine."), "It's fine.");
    }

    #[test]
    fn hunks_are_individual_fixes() {
        let o = "hey i was thinking about it yesterday honestly i dont know";
        let f = "Hey, I was thinking about it yesterday. Honestly, I don't know.";
        let h = hunks(o, f);
        let pairs: Vec<(&str, &str, &str)> = h.iter().map(|x| (x.orig.as_str(), x.repl.as_str(), x.category)).collect();
        assert_eq!(pairs, vec![
            ("hey", "Hey,", "Punctuation"),
            ("i", "I", "Capitalization"),
            ("yesterday", "yesterday.", "Punctuation"),
            ("honestly", "Honestly,", "Punctuation"),
            ("i", "I", "Capitalization"),
            ("dont", "don't", "Spelling"),
            ("know", "know.", "Punctuation"),
        ]);
        // Applying any one fix changes exactly that word.
        assert_eq!(apply_hunk(o, &h[5]), "hey i was thinking about it yesterday honestly i don't know");
        // Applying all of them, right to left, gives the full correction.
        let mut t = o.to_string();
        for x in h.iter().rev() {
            t = apply_hunk(&t, x);
        }
        assert_eq!(t, f);
    }

    #[test]
    fn hunks_handle_insertions_removals_and_rewording() {
        let h = hunks("I would of helped", "I would have helped");
        assert_eq!((h[0].orig.as_str(), h[0].repl.as_str(), h[0].category), ("of", "have", "Grammar"));
        let h = hunks("before and it went", "before, and it went");
        assert_eq!((h[0].orig.as_str(), h[0].repl.as_str()), ("before", "before,"));
        let h = hunks("this is is fine", "this is fine");
        assert_eq!(apply_hunk("this is is fine", &h[0]), "this is fine");
        let h = hunks("see you tomorow", "see you tomorrow");
        assert_eq!(h[0].category, "Spelling");
        assert!(hunks("same text.", "same text.").is_empty());
    }

    #[test]
    fn composition_splits_around_current_paragraph() {
        let c = composition("Hi team,\n\n  next week we fix it", " and more");
        assert_eq!(c.before, "Hi team,\n\n  ");
        assert_eq!(c.target, "next week we fix it");
        assert_eq!(c.after, " and more");
        assert!(!c.ends_sentence);
        let c = composition("hey. how are you? ", "");
        assert_eq!(c.target, "hey. how are you? ");
        assert!(c.ends_sentence);
    }

    #[test]
    fn invented_endings_trimmed() {
        let s = "I dont think thats a good idea because its";
        assert_eq!(trim_invented_ending(s, "I don't think that's a good idea because it is.").as_deref(), Some("I don't think that's a good idea because it is"));
        assert_eq!(trim_invented_ending(s, "I don't think that's a good idea because it seems risky."), None);
        assert_eq!(trim_invented_ending("Done.", "Done!").as_deref(), Some("Done!"));
        assert_eq!(normalize_completion("I ", " _______ my car"), None);
    }

    #[test]
    fn trailing_ws_kept() {
        assert_eq!(keep_trailing_ws("their going. ", "They're going."), "They're going. ");
    }
}
