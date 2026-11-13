# Writing quality: investigation and results (2026-11-13)

Suite: `tests/quality_cases.json`, 32 cases in 14 categories: apostrophes, contractions, commas, periods, questions, direct speech, long multi-sentence messages, multiple paragraphs, run-ons, incomplete sentences, casual writing, mixed English/Tagalog, already-correct text, and context-dependent words.

Runner: `cargo run --release --example quality -- <model.gguf> [cpu]`. It uses the app's exact pipeline. Full outputs are in `bench/quality-*.md`.

* **Correct mode** is graded against accepted answers. A case passes only if the output matches one of them **and** keeps required tokens ("u guys", "sige", "rn") **and** avoids forbidden ones ("you guys", unwanted quotes).
* **Improve and Rephrase** are graded by the shipped validator plus the keep/must-not rules. Every output was also read by hand for meaning.

## What was wrong (v0.1 baseline: 5/32 in Correct)

Measured with `quality-qwen2.5-1.5b-instruct-q4_k_m-baseline.md`.

| Cause | Evidence | Fix |
|---|---|---|
| Unfinished-sentence guard stripped final punctuation from **every** text without `.?!`. The pipeline couldn't tell "still typing" from "finished thought". | ~12 failures: "That's actually pretty good" without its period | Three passes. *Settle* (700 ms) protects an unfinished last sentence. *Idle* (2 s) and the shortcut treat the text as finished. The STATUS line states this explicitly. |
| Only the last sentence was sent, with no surrounding message. | "there place is huge" became "*There's a place that's* huge" | The model sees the whole field: the current paragraph as TEXT, plus up to 800 characters of earlier paragraphs and 400 after the cursor as read-only CONTEXT. |
| The validator only checked length. | Accepted "they're owner" → "**the** owner", "u" → "you", and dropping "sige" | Word-alignment validator (`src/validate.rs`), described below. |
| One generic one-line prompt with no punctuation rules. | "yeah, i was thinking", missing "?" | Separate instructions per mode with an explicit punctuation checklist, plus examples not taken from the suite. |
| Harper's instant findings never reached the model. | The model missed "i" → "I" while Harper caught it in 3 ms | Harper's findings go to the model as CHECKER NOTES. |
| Free-text replies. | Risk of "Here is the corrected text: …" | JSON output constrained by schema (llama.cpp grammar), then parsed and validated. |
| No mid-generation cancel; no check at accept time; one cache slot. | Stale work kept running; text could change before Tab | Streaming with cancel (closing the connection aborts generation); the field is re-read and compared before inserting; one cache slot per request kind. |

## Validator (applied before anything is shown)

* Words are aligned after removing case and punctuation, so `dont` = `don't` and punctuation fixes cost nothing. Every original word the model drops must be explained by one of:
  * a swap within a commonly confused group (there/their/they're, to/too, of/have…);
  * Harper's own suggestion for that word;
  * an article.
* A correctly spelled word may never be "respelled" (sarahs → Sara's is rejected).
* Every mode must keep: names, numbers, links, @mentions, emoji, casual short forms (u, rn, lol, gonna…), and words Harper identified as another language.
* Meaning guards: negation must stay present or absent, and thanks or apologies can't disappear.
* An unfinished sentence may not gain words after its last word, and any final punctuation added to it is removed.

## Results (same suite, same rules)

| | Correct | Improve | Rephrase | Correct median, GPU | Correct median, CPU only |
|---|---|---|---|---|---|
| v0.1 pipeline, Qwen2.5-1.5B | **5/32** | n/a | n/a | 246 ms | n/a |
| New pipeline, Qwen2.5-1.5B | **21/32** | 27/32 | 27/30 | 208 ms | 1.36 s |
| New pipeline, Qwen3.5-2B | 22/32 | 22/32 | 26/30 | 437 ms | not run |
| New pipeline, **Qwen3-4B-Instruct-2507** | **30/32** | 26/32 | 26/30 | 440 ms | 4.5 s |

GPU: RTX 4050 Laptop (Vulkan). CPU: i7-13620H, 8 threads. A correction right after a prediction (the app's real pattern) takes 155 ms (1.5B GPU), 321 ms (4B GPU) and 540 ms (1.5B CPU). With four cache slots, alternating request kinds no longer re-reads the instructions.

**Model choice:** automatic. With a GPU that has ≥ 6 GB of dedicated memory, Margin uses Qwen3-4B. Otherwise it uses Qwen2.5-1.5B, because 4B on CPU (4.5 s) is too slow for suggestions while typing. Both can be chosen in Settings.

**What model size fixes:** the 1.5B model still misses "your going" → "You're", "there going" → "They're", "Friday." → "Friday?", and keeps "?" on an indirect question. 4B gets all of these except the indirect question.

## Known remaining issues (4B)

* "I wonder if she got my message?" keeps its question mark. All three models do this.
* Mixed language: "Ok sige …" ends without a period, and Improve capitalizes "Sige" mid-sentence. The meaning is intact.
* Improve and Rephrase sometimes produce near-identical alternatives. Duplicates are removed, so sometimes only one row appears.

## Desktop verification (real app, `examples/e2e.rs app`)

* "I dont think thats a good idea because its " → Tab → "I don't think that's a good idea because it's". The sentence is unfinished, so no period is added.
* The spec's run-on message, typed in one go with a 4.5 s pause → Tab → "Hey, I was thinking about what you said yesterday. Honestly, I don't think that's going to work because we already tried something similar before and it didn't really go well."
* The Win32, RichEdit, Edge textarea, contenteditable, chat-editor and password cases all still pass.
