use harper_core::linting::{LintGroup, Linter};
use harper_core::parsers::PlainEnglish;
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};
use std::time::Instant;

fn main() {
    let t = Instant::now();
    let dict = FstDictionary::curated();
    let mut linter = LintGroup::new_curated(dict.clone(), Dialect::American);
    println!("init: {:?}", t.elapsed());
    for text in [
        "I dont think thats a good idea because its",
        "their going to the store tomorow and i will meet them their.",
        "She dont know what she want.",
        "This is an test of the the system.",
    ] {
        let t = Instant::now();
        let doc = Document::new_curated(text, &PlainEnglish);
        let lints = linter.lint(&doc);
        println!("\n{text}\n  {} lints in {:?}", lints.len(), t.elapsed());
        for l in lints {
            let span: String = doc.get_source()[l.span.start..l.span.end].iter().collect();
            println!("  [{}..{}] '{}' {:?}: {} -> {:?}", l.span.start, l.span.end, span, l.lint_kind, l.message, l.suggestions);
        }
    }
}
