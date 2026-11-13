//! Writing-quality evaluation against tests/quality_cases.json, using exactly
//! the app's pipeline: Composition → Harper analysis → mode prompt → validation.
//!
//!   cargo run --release --example quality -- <model-file.gguf> [cpu]
//!
//! Correct mode is graded against accepted answers. Improve and Rephrase are
//! graded by the shipped validator plus per-case keep/must_not rules; every
//! output goes into bench/quality-<model>.md for human review of meaning.
//! (The v0.1 baseline report is bench/quality-qwen2.5-1.5b-instruct-q4_k_m-baseline.md.)

use margin::grammar::Grammar;
use margin::inference::{self, Mode, OpenAiCompat, Provider, Revision, Style};
use margin::runtime::LocalServer;
use margin::text;
use serde::Deserialize;
use std::time::Instant;

#[derive(Deserialize)]
struct Case {
    id: String,
    cat: String,
    text: String,
    #[serde(default)]
    before: String,
    finished: bool,
    correct: Vec<String>,
    #[serde(default)]
    keep: Vec<String>,
    #[serde(default)]
    must_not: Vec<String>,
}

fn norm(s: &str) -> String {
    s.replace(['“', '”'], "\"").replace(['‘', '’'], "'").split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Run<'a> {
    p: &'a OpenAiCompat,
    g: Grammar,
    style: Style,
}

impl Run<'_> {
    /// The app's pipeline for one case and mode. Returns candidates or why none.
    fn go(&mut self, c: &Case, mode: Mode) -> Result<Vec<String>, String> {
        let comp = text::composition(&format!("{}{}", c.before, c.text), "");
        let a = self.g.analyze(&comp.target, c.finished);
        let r = Revision {
            before: &comp.before,
            target: &comp.target,
            after: &comp.after,
            finished: c.finished,
            hints: &a.hints,
            flagged: &a.flagged,
            mixed_language: a.mixed_language,
            foreign: &a.foreign,
            rules_ran: true,
            style: self.style,
        };
        inference::suggest(self.p, mode, &r, &|| false)
    }
}

fn rules_ok(c: &Case, out: &str) -> Result<(), String> {
    for k in &c.keep {
        if !out.contains(k.as_str()) {
            return Err(format!("lost {k:?}"));
        }
    }
    for m in &c.must_not {
        if out.contains(m.as_str()) {
            return Err(format!("contains {m:?}"));
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let model_file = args.get(1).expect("model file name");
    let gpu = !args.iter().any(|a| a == "cpu");
    let dir = margin::runtime::data_dir();
    let server = LocalServer::start(&dir.join("runtime/llama-server.exe"), &dir.join("models").join(model_file), gpu).expect("server");
    let p = OpenAiCompat::new(&format!("http://127.0.0.1:{}", server.port), "", None, true);
    margin::engine::warm_up(&p);
    let interleaved = interleaved_latency(&p);
    let mut run = Run { p: &p, g: Grammar::new(harper_core::Dialect::American), style: Style::Standard };

    let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/quality_cases.json")).unwrap()).unwrap();
    if args.iter().any(|a| a == "styles") {
        // Rephrase in every style, on a varied subset, for reading side by side.
        let ids = ["contr-1", "comma-2", "period-1", "long-1", "long-2", "run-1", "casual-2", "ctx-2", "ctx-3", "mixed-2"];
        let mut out = format!("# Rephrase styles: {model_file}\n\n");
        let mut summary = vec![];
        for st in Style::ALL {
            run.style = st;
            let (mut ok, mut ms) = (0, vec![]);
            out += &format!("## {}\n\n", st.label());
            for c in cases.iter().filter(|c| ids.contains(&c.id.as_str())) {
                let t = Instant::now();
                let r = run.go(c, Mode::Rephrase);
                ms.push(t.elapsed().as_millis());
                let line = match r {
                    Ok(v) if !v.is_empty() => {
                        ok += 1;
                        v.iter().map(|o| format!("`{o}`")).collect::<Vec<_>>().join(" / ")
                    }
                    Ok(_) => "(nothing)".into(),
                    Err(e) => format!("(rejected: {e})"),
                };
                out += &format!("- **{}** `{}`\n  - {line}\n", c.id, c.text);
            }
            ms.sort();
            summary.push(format!("{}: {ok}/{} usable, median {} ms", st.label(), ids.len(), ms[ms.len() / 2]));
        }
        out += &format!("\n**{}**\n", summary.join("; "));
        let path = format!("{}/bench/styles-{}.md", env!("CARGO_MANIFEST_DIR"), model_file.trim_end_matches(".gguf"));
        std::fs::write(&path, &out).unwrap();
        println!("{}\n -> {path}", summary.join("\n"));
        return;
    }
    let label = format!("{model_file} ({})", if gpu { "GPU" } else { "CPU only" });
    let mut out = format!("# {label}\n\n## Correct (graded against accepted answers)\n\n");
    let (mut pass, mut ms) = (0, Vec::new());
    let mut fails_by_cat: Vec<String> = Vec::new();

    for c in &cases {
        let t = Instant::now();
        let r = run.go(c, Mode::Correct);
        ms.push(t.elapsed().as_millis());
        // Empty Ok = the model judged the text correct (unchanged).
        let result = match &r {
            Ok(v) => v.first().cloned().unwrap_or_else(|| c.text.clone()),
            Err(e) => format!("(rejected: {e})"),
        };
        let ok = c.correct.iter().any(|a| norm(a) == norm(&result)) && rules_ok(c, &result).is_ok();
        if ok {
            pass += 1;
        } else {
            fails_by_cat.push(c.cat.clone());
        }
        out += &format!("- {} **{}** [{}] {} ms\n  - in:  `{}`\n  - out: `{}`\n", if ok { "PASS" } else { "FAIL" }, c.id, c.cat, ms.last().unwrap(), c.text, result);
        if !ok {
            out += &format!("  - want: `{}`\n", c.correct[0]);
        }
    }
    ms.sort();
    let summary_correct = format!("Correct {pass}/{} (median {} ms, p90 {} ms)", cases.len(), ms[ms.len() / 2], ms[ms.len() * 9 / 10]);
    out += &format!("\n**{summary_correct}**\n");

    let mut summaries = vec![summary_correct];
    for mode in [Mode::Improve, Mode::Rephrase] {
        out += &format!("\n## {} (validator + keep/must-not rules; read for meaning)\n\n", mode.label());
        let (mut produced, mut total, mut ms) = (0, 0, Vec::new());
        for c in cases.iter().filter(|c| mode != Mode::Rephrase || c.finished) {
            total += 1;
            let t = Instant::now();
            let r = run.go(c, mode);
            ms.push(t.elapsed().as_millis());
            let line = match r {
                Ok(v) if !v.is_empty() => {
                    let bad: Vec<String> = v.iter().filter_map(|o| rules_ok(c, o).err()).collect();
                    if bad.is_empty() {
                        produced += 1;
                    }
                    format!("{}{}", v.iter().map(|o| format!("`{o}`")).collect::<Vec<_>>().join(" / "), if bad.is_empty() { String::new() } else { format!("  **RULE FAIL: {}**", bad.join(", ")) })
                }
                Ok(_) => "(no change suggested)".into(),
                Err(e) => format!("(rejected: {e})"),
            };
            out += &format!("- **{}** {} ms\n  - in:  `{}`\n  - out: {line}\n", c.id, ms.last().unwrap(), c.text);
        }
        ms.sort();
        let s = format!("{} {produced}/{total} usable (median {} ms, p90 {} ms)", mode.label(), ms[ms.len() / 2], ms[ms.len() * 9 / 10]);
        out += &format!("\n**{s}**\n");
        summaries.push(s);
    }
    let path = format!("{}/bench/quality-{}{}.md", env!("CARGO_MANIFEST_DIR"), model_file.trim_end_matches(".gguf"), if gpu { "" } else { "-cpu" });
    out += &format!("\n## App-like interleaving\n\n{interleaved}\n");
    std::fs::write(&path, &out).unwrap();
    println!("{label}\n  {}\n  {interleaved}\n  Correct failures by category: {:?}\n  -> {path}", summaries.join("\n  "), fails_by_cat);
}

/// The app alternates prediction and correction on every pause. Measure the
/// correction latency in that pattern (cache slots make the difference).
fn interleaved_latency(p: &OpenAiCompat) -> String {
    let texts = ["so i was wondering if you", "thanks for the help yesterday it really", "can we move the call to", "i think the new design looks", "let me know when your"];
    let mut ms = Vec::new();
    for t in texts {
        let _ = p.complete(t);
        let s = Instant::now();
        let r = Revision { before: "", target: t, after: "", finished: false, hints: &[], flagged: &[], mixed_language: false, foreign: &[], rules_ran: false, style: margin::inference::Style::Standard };
        let _ = p.revise(Mode::Correct, &r, &|| false);
        ms.push(s.elapsed().as_millis());
    }
    ms.sort();
    format!("Correct right after a prediction: median {} ms, max {} ms", ms[ms.len() / 2], ms[ms.len() - 1])
}
