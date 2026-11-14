//! Tier 2: language-model features behind a provider-neutral trait.
//! The engine only sees `Provider`; swapping models or vendors means adding an
//! implementation here, nothing else.
//!
//! Every revision request carries the whole composition: the paragraph being
//! edited (TEXT) plus the rest of the field as read-only CONTEXT, whether the
//! last sentence is still being typed, and the rule checker's findings.
//! Output is constrained to a JSON schema and validated (see `validate`).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::BufRead;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
    /// Grammar, spelling, punctuation. Keeps the writer's words.
    Correct,
    /// Flow and clarity. Keeps meaning and conversational tone.
    Improve,
    /// Same idea, different wording.
    Rephrase,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Correct => "Correct",
            Mode::Improve => "Improve",
            Mode::Rephrase => "Rephrase",
        }
    }
}

/// How Rephrase alternatives should sound (QuillBot-style modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Style {
    #[default]
    Standard,
    Formal,
    Corporate,
    Simple,
    Creative,
    Academic,
}

impl Style {
    pub const ALL: [Style; 6] = [Style::Standard, Style::Formal, Style::Corporate, Style::Simple, Style::Creative, Style::Academic];

    pub fn label(self) -> &'static str {
        match self {
            Style::Standard => "Standard",
            Style::Formal => "Formal",
            Style::Corporate => "Corporate",
            Style::Simple => "Simple",
            Style::Creative => "Creative",
            Style::Academic => "Academic",
        }
    }

    /// Goes in the request (not the system prompt) so each mode's cached
    /// instructions are reused whatever the style.
    fn instruction(self) -> &'static str {
        match self {
            Style::Standard => "STYLE: standard. Keep the writer's own tone and level of formality; casual stays casual.",
            Style::Formal => "STYLE: formal. Polished and professional, suitable for a work email: no slang or text-speak, no contractions, complete sentences, courteous wording.",
            Style::Corporate => "STYLE: corporate. Professional workplace tone for colleagues, managers and clients: clear, confident, courteous and action-oriented. Lead with the point, use plain business language (no slang, no jargon or buzzwords), and keep it concise. Contractions are fine.",
            Style::Simple => "STYLE: simple. Plain and easy to read: short sentences and common everyday words, no jargon. Anyone should understand it at first read.",
            Style::Creative => "STYLE: creative. Vivid and expressive: fresh word choices and livelier phrasing. Say the same thing; do not add new facts, events or opinions.",
            Style::Academic => "STYLE: academic. Scholarly and precise: objective tone, formal vocabulary, no contractions or slang. Keep \"I\" if the writer talks about themselves.",
        }
    }
}

/// One revision request: the paragraph before the caret plus its surroundings.
pub struct Revision<'a> {
    /// Earlier paragraphs of the same message (read-only context).
    pub before: &'a str,
    /// The paragraph to revise, up to the caret.
    pub target: &'a str,
    /// Text after the caret (read-only context).
    pub after: &'a str,
    /// False while the writer is still typing the last sentence.
    pub finished: bool,
    /// Rule-checker findings, e.g. "dont → don't".
    pub hints: &'a [String],
    /// (original, suggestion) pairs from the rule checker (for validation).
    pub flagged: &'a [(String, String)],
    /// The text mixes English with another language.
    pub mixed_language: bool,
    /// Words from another language that must be kept.
    pub foreign: &'a [String],
    /// The rule checker ran on `target` (so unflagged words are spelled correctly).
    pub rules_ran: bool,
    /// Style for Rephrase alternatives.
    pub style: Style,
}

pub trait Provider: Send {
    /// Short label for the UI, e.g. "On this PC" or "api.example.com".
    fn label(&self) -> String;
    /// True when text never leaves this computer.
    fn is_local(&self) -> bool;
    /// Raw candidates for `mode` (validated by the caller). `cancel` is polled
    /// while generating; returning true aborts the request.
    fn revise(&self, mode: Mode, r: &Revision, cancel: &dyn Fn() -> bool) -> Result<Vec<String>, String>;
    /// Predict the next few words after `context`.
    fn complete(&self, context: &str) -> Result<String, String>;
}

/// Revise and keep only candidates that pass the meaning-preservation checks.
pub fn suggest(p: &dyn Provider, mode: Mode, r: &Revision, cancel: &dyn Fn() -> bool) -> Result<Vec<String>, String> {
    let raw = p.revise(mode, r, cancel)?;
    let mut out: Vec<String> = Vec::new();
    let mut last_err = String::new();
    for c in raw {
        let rules = crate::validate::Rules { flagged: r.flagged, foreign: r.foreign, ran: r.rules_ran, style: r.style };
        match crate::validate::check_with(mode, r.target, &c, r.finished, &rules) {
            Ok(v) if !out.contains(&v) => out.push(v),
            Ok(_) => {}
            Err(e) => last_err = format!("{e:?}: {c:?}"),
        }
    }
    if out.is_empty() && !last_err.is_empty() { Err(last_err) } else { Ok(out) }
}

// ---------------------------------------------------------------------------
// Instructions. Each mode has its own task definition, rules and examples.
// Examples are deliberately unlike the test suite (tests/quality_cases.json).
// ---------------------------------------------------------------------------

const SHARED_INPUT: &str = "\
Input format:
- TEXT is what you work on.
- CONTEXT BEFORE / CONTEXT AFTER are other parts of the same message. Use them to understand the meaning and tone, never copy or change them.
- STATUS is `complete`, or `unfinished` when the writer is still typing the last sentence of TEXT.
- CHECKER NOTES come from a spell checker. They are often right about apostrophes and capital letters, but can be wrong about names, slang and words from other languages.
Reply with JSON only. No explanations, no markdown.";

const SYS_CORRECT: &str = "\
You are a careful proofreader for everyday messages: chats, emails, comments and posts.
Task: return TEXT with its spelling, grammar, capitalization and punctuation corrected, and nothing else changed.

Work in this order: read TEXT and its context, work out what the writer means, find each error, then fix it.

Check every sentence for:
1. Capital letters: the first word of every sentence, the word \"I\", names of people and places, days and months. \
Keep every name spelled exactly as written; only fix its capital letter.
2. Apostrophes: contractions (don't, can't, I'm, it's, you're, they're, we're) and possessives (Anna's). \
\"its\" means belonging to it; \"it's\" means it is or it has. Choose there/their/they're, your/you're, then/than, \
to/too, affect/effect and similar pairs by their meaning in this sentence.
3. Sentence boundaries: when two complete sentences run together without punctuation, split them with a period \
or join them with a comma plus and/but/so. End every complete sentence with a period, question mark or exclamation mark.
4. Questions: a direct question ends with \"?\". A statement that only mentions a question (\"I wonder if...\", \
\"she asked whether...\") ends with a period.
5. Commas: after an opening word such as Yeah, Hey, Honestly, Thanks, So or Well; around the name of the person \
being addressed; after an opening clause (\"If you can, ...\"); before and/but/so when they join two complete sentences.
6. Quotation marks only around words someone actually said, and only when the writer is quoting them. \
Use colons, semicolons and parentheses only where they are clearly needed.

Preserve the writer's voice:
- Keep their words. Do not replace words with synonyms and do not reword sentences that are already correct.
- Keep casual style: contractions, slang and short forms (u, rn, lol, gonna, idk) stay exactly as written.
- Keep words from other languages exactly as written.
- Do not add or remove ideas, greetings or sign-offs.
If STATUS is unfinished: fix the last sentence too, but do not end it with punctuation and do not add any words after its last word.
If TEXT is already correct, return it unchanged.

Reply as {\"text\": \"<corrected TEXT>\"}.";

const SYS_IMPROVE: &str = "\
You are an editor for everyday messages: chats, emails, comments and posts.
Task: return TEXT rewritten so it reads clearly and naturally, while saying exactly the same thing.

Work in this order: read TEXT and its context, work out what the writer means and who they are talking to, \
then improve it.

Do:
- Fix every spelling, grammar, capitalization and punctuation error (apostrophes, commas, sentence-ending marks, question marks).
- Smooth awkward phrasing and clumsy sentence structure.
- Remove words that are repeated without purpose.
- Improve how sentences connect, for example by joining short choppy sentences or splitting a long run-on one.
Do not:
- Add new information, opinions, greetings or sign-offs, or drop anything the writer said.
- Make a casual message formal. Keep contractions, friendly words and the writer's level of politeness.
- Change slang or short forms: u, rn, lol, gonna, idk and similar stay exactly as written.
- Change names, numbers, dates or words from other languages.
- Change who is thanking, apologizing, asking, agreeing or refusing.
- Turn direct speech into reported speech. If someone's words are quoted, keep them as their words.
- Rewrite more than needed: if a sentence is already clear, keep it.
If STATUS is unfinished, keep the last sentence unfinished: no final punctuation and no added words after its last word.

Reply as {\"text\": \"<improved TEXT>\"}.";

const SYS_REPHRASE: &str = "\
You are a writing assistant that offers other ways to say the same thing.
Task: write 2 alternative versions of TEXT.

Work in this order: read TEXT and its context, work out exactly what the writer means, then express that same \
meaning in different words.

Each alternative must:
- Say the same thing with the same intent: no new information, no missing information.
- Use noticeably different wording or sentence structure from TEXT and from the other alternative.
- Read naturally and be correctly spelled and punctuated.
- Follow STYLE. With STYLE standard, match the tone of the original: casual stays casual, polite stays polite.
- Keep names, numbers, dates and words from other languages. With STYLE standard also keep slang and short forms \
such as u, rn, lol and gonna; other styles may write them out.
- Keep who is doing what: whoever thanks, apologizes, asks, agrees or refuses in TEXT does so in the alternative too.
- Keep negations: if TEXT says something is not so, the alternative says that too.
- When TEXT reports what someone said (\"she said i will be there\"), the speaker of those words stays the same: \
\"she said she would be there\", never \"she said I would be there\".

Reply as {\"alternatives\": [\"<first>\", \"<second>\"]}.";

fn shots(mode: Mode) -> &'static [(&'static str, &'static str)] {
    match mode {
        Mode::Correct => &[
            (
                "TEXT:\nlol yeah thats what i said, see u at 8\n\nSTATUS: complete\nCHECKER NOTES (may be wrong): thats → that's; i → I",
                r#"{"text": "Lol, yeah, that's what I said. See u at 8."}"#,
            ),
            (
                "CONTEXT BEFORE (do not change):\nThanks for the invite!\n\nTEXT:\nim not sure i can make it my sister is visiting this weekend and i promised to pick her up from the airport\n\nSTATUS: complete",
                r#"{"text": "I'm not sure I can make it. My sister is visiting this weekend, and I promised to pick her up from the airport."}"#,
            ),
            (
                "TEXT:\ndid you get a chance to look at marks email about the\n\nSTATUS: unfinished",
                r#"{"text": "Did you get a chance to look at Mark's email about the"}"#,
            ),
            ("TEXT:\nSige, I'll call you later.\n\nSTATUS: complete", r#"{"text": "Sige, I'll call you later."}"#),
            // there/their/they're and possessives of day names: measured misses.
            (
                "TEXT:\nthere both good options but i think mondays plan is better\n\nSTATUS: complete",
                r#"{"text": "They're both good options, but I think Monday's plan is better."}"#,
            ),
        ],
        Mode::Improve => &[(
            "TEXT:\nso basically the thing is that i cant come tomorrow, i cant come because i have work, sorry about that\n\nSTATUS: complete",
            r#"{"text": "Sorry, I can't come tomorrow because I have work."}"#,
        )],
        Mode::Rephrase => &[(
            "TEXT:\nI can't make it tonight, something came up at work.\n\nSTATUS: complete\nSTYLE: standard. Keep the writer's own tone and level of formality; casual stays casual.",
            r#"{"alternatives": ["Something came up at work, so I won't be able to make it tonight.", "I have to skip tonight because of something at work."]}"#,
        )],
    }
}

/// Spelled out because "complete" alone left final punctuation off (measured).
const STATUS_DONE: &str = "STATUS: complete (the writer has finished, so every sentence, including the last, ends with . ? or !)";
const STATUS_OPEN: &str = "STATUS: unfinished (the writer is still typing the last sentence: no final punctuation, no added words)";

/// Examples are written with the short status; expand it so they match real requests.
fn expand_status(s: &str) -> String {
    s.replace("STATUS: complete", STATUS_DONE).replace("STATUS: unfinished", STATUS_OPEN)
}

pub fn user_message(r: &Revision) -> String {
    let mut s = String::new();
    if !r.before.trim().is_empty() {
        s += "CONTEXT BEFORE (do not change):\n";
        s += r.before.trim();
        s += "\n\n";
    }
    s += "TEXT:\n";
    s += r.target.trim();
    s += "\n\n";
    if !r.after.trim().is_empty() {
        s += "CONTEXT AFTER (do not change):\n";
        s += r.after.trim();
        s += "\n\n";
    }
    s += if r.finished { STATUS_DONE } else { STATUS_OPEN };
    if r.mixed_language {
        s += "\nNOTE: this message mixes English with another language. Keep the non-English words exactly as written.";
    }
    if !r.hints.is_empty() {
        s += "\nCHECKER NOTES (may be wrong): ";
        s += &r.hints.join("; ");
    }
    s
}

fn schema(mode: Mode) -> Value {
    match mode {
        Mode::Rephrase => json!({"type": "object", "properties": {"alternatives": {"type": "array", "items": {"type": "string"}, "minItems": 2, "maxItems": 2}}, "required": ["alternatives"], "additionalProperties": false}),
        _ => json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"], "additionalProperties": false}),
    }
}

/// Parse `{"text": ...}` / `{"alternatives": [...]}`, tolerating stray text around the JSON.
pub fn parse_reply(mode: Mode, raw: &str) -> Result<Vec<String>, String> {
    let s = raw.trim();
    let json_part = match (s.find('{'), s.rfind('}')) {
        (Some(a), Some(b)) if b > a => &s[a..=b],
        _ => return Err(format!("no JSON in reply: {s:?}")),
    };
    let v: Value = serde_json::from_str(json_part).map_err(|e| format!("bad JSON ({e}): {json_part:?}"))?;
    let out: Vec<String> = match mode {
        Mode::Rephrase => v["alternatives"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        _ => v["text"].as_str().map(|t| vec![t.to_string()]).unwrap_or_default(),
    };
    if out.is_empty() { Err(format!("reply missing field: {json_part:?}")) } else { Ok(out) }
}

/// OpenAI-compatible chat endpoint. Used both for the local llama.cpp server
/// (which also offers raw `/completion` for fast prediction) and for any
/// remote endpoint the user configures.
pub struct OpenAiCompat {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub local: bool,
    agent: ureq::Agent,
}

impl OpenAiCompat {
    pub fn new(base_url: &str, model: &str, api_key: Option<String>, local: bool) -> Self {
        let timeout = if local { 30 } else { 15 };
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(timeout)))
            .https_only(!local)
            .http_status_as_error(false)
            .build()
            .new_agent();
        OpenAiCompat { base_url: base_url.trim_end_matches('/').to_string(), model: model.into(), api_key, local, agent }
    }

    fn request(&self, path: &str, body: &Value) -> Result<ureq::http::Response<ureq::Body>, String> {
        let mut req = self.agent.post(format!("{}{}", self.base_url, path)).header("content-type", "application/json");
        if let Some(k) = &self.api_key {
            req = req.header("authorization", format!("Bearer {k}"));
        }
        req.send_json(body).map_err(|e| e.to_string())
    }

    /// Streamed chat completion; stops (and closes the connection, which makes
    /// llama.cpp abort generation) as soon as `cancel()` returns true.
    fn chat(&self, mode: Mode, user: &str, max_tokens: u32, temperature: f32, cancel: &dyn Fn() -> bool) -> Result<String, String> {
        let system = match mode {
            Mode::Correct => SYS_CORRECT,
            Mode::Improve => SYS_IMPROVE,
            Mode::Rephrase => SYS_REPHRASE,
        };
        let mut messages = vec![json!({"role": "system", "content": format!("{system}\n\n{SHARED_INPUT}")})];
        for (u, a) in shots(mode) {
            messages.push(json!({"role": "user", "content": expand_status(u)}));
            messages.push(json!({"role": "assistant", "content": a}));
        }
        messages.push(json!({"role": "user", "content": user}));
        let mut body = json!({
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "stream": true,
            "response_format": {"type": "json_schema", "json_schema": {"name": "reply", "strict": true, "schema": schema(mode)}},
        });
        if !self.model.is_empty() {
            body["model"] = json!(self.model);
        }
        if self.local {
            body["chat_template_kwargs"] = json!({"enable_thinking": false});
            body["cache_prompt"] = json!(true);
        }
        let mut resp = self.request("/v1/chat/completions", &body)?;
        if resp.status().as_u16() == 400 && !self.local {
            // Some providers don't support json_schema; the prompt still asks for JSON.
            body.as_object_mut().unwrap().remove("response_format");
            resp = self.request("/v1/chat/completions", &body)?;
        }
        if !resp.status().is_success() {
            let code = resp.status();
            let text = resp.body_mut().read_to_string().unwrap_or_default();
            return Err(format!("HTTP {code}: {}", text.chars().take(200).collect::<String>()));
        }
        let reader = std::io::BufReader::new(resp.into_body().into_reader());
        let mut out = String::new();
        for line in reader.lines() {
            if cancel() {
                return Err("cancelled".into());
            }
            let line = line.map_err(|e| e.to_string())?;
            let Some(data) = line.strip_prefix("data: ") else { continue };
            if data.trim() == "[DONE]" {
                break;
            }
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
                    out.push_str(t);
                }
            }
        }
        Ok(out)
    }
}

impl Provider for OpenAiCompat {
    fn label(&self) -> String {
        if self.local {
            "On this PC".into()
        } else {
            self.base_url.split("//").nth(1).unwrap_or(&self.base_url).split('/').next().unwrap_or("").to_string()
        }
    }

    fn is_local(&self) -> bool {
        self.local
    }

    fn revise(&self, mode: Mode, r: &Revision, cancel: &dyn Fn() -> bool) -> Result<Vec<String>, String> {
        let len = r.target.chars().count() as u32;
        let (max, temp) = match mode {
            Mode::Correct => (len / 2 + 40, 0.0),
            Mode::Improve => (len / 2 + 48, 0.3),
            Mode::Rephrase => (len * 3 / 2 + 64, 0.7),
        };
        let mut user = user_message(r);
        if mode == Mode::Rephrase {
            user += "\n";
            user += r.style.instruction();
        }
        let raw = self.chat(mode, &user, max, temp, cancel)?;
        parse_reply(mode, &raw)
    }

    fn complete(&self, context: &str) -> Result<String, String> {
        if self.local {
            // Raw continuation is both faster and more natural than chat for prediction.
            let mut resp = self.request(
                "/completion",
                &json!({"prompt": context, "n_predict": 8, "temperature": 0.0, "cache_prompt": true,
                        "stop": ["\n", ".", "!", "?", "\""]}),
            )?;
            let v: Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
            v["content"].as_str().map(str::to_string).ok_or_else(|| "bad completion".into())
        } else {
            let body = json!({
                "messages": [
                    {"role": "system", "content": "Continue the user's text with the next 1 to 4 words they are most likely to type. Reply with only those words."},
                    {"role": "user", "content": context}],
                "max_tokens": 8, "temperature": 0.0, "model": self.model,
            });
            let mut resp = self.request("/v1/chat/completions", &body)?;
            let v: Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
            v["choices"][0]["message"]["content"].as_str().map(|s| format!(" {}", s.trim())).ok_or_else(|| "bad completion".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_structured_replies() {
        assert_eq!(parse_reply(Mode::Correct, r#"{"text": "Hi, Sam."}"#).unwrap(), vec!["Hi, Sam."]);
        assert_eq!(parse_reply(Mode::Correct, "Sure! {\"text\": \"Hi.\"} hope it helps").unwrap(), vec!["Hi."]);
        assert_eq!(parse_reply(Mode::Rephrase, r#"{"alternatives": ["A.", "B."]}"#).unwrap(), vec!["A.", "B."]);
        assert!(parse_reply(Mode::Correct, "Hi.").is_err());
        assert!(parse_reply(Mode::Correct, r#"{"foo": 1}"#).is_err());
    }

    #[test]
    fn user_message_carries_context_and_status() {
        let hints = vec!["dont → don't".to_string()];
        let r = Revision { before: "Hi team,\n", target: "i dont know", after: "", finished: false, hints: &hints, flagged: &[], mixed_language: false, foreign: &[], rules_ran: true, style: Style::Standard };
        let m = user_message(&r);
        assert!(m.starts_with("CONTEXT BEFORE (do not change):\nHi team,\n\nTEXT:\ni dont know\n\nSTATUS: unfinished ("));
        assert!(m.ends_with("CHECKER NOTES (may be wrong): dont → don't"));
    }
}
