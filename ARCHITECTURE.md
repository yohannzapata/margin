# Margin: architecture and decisions

Margin is a background Windows app that offers predictive text, grammar and spelling fixes, and rephrasing in the text field you are typing in. It shows them in a small popup at the caret and never takes focus.

## 1. Research findings (verified 2026-11-13)

| Question | Finding | Consequence |
|---|---|---|
| Is there a free cloud model API we can ship? | Free tiers (Gemini, Groq, HF Inference) need a per-user key, are rate-limited, and change terms. A key embedded in a desktop app gets extracted and abused. | **No bundled cloud dependency.** Remote inference is opt-in, uses the user's own server, and is disclosed. |
| Is browser built-in AI usable? | Chrome's Proofreader and Rewriter APIs were origin or developer trials in the latest docs found. They need large hardware (Gemini Nano) and only work inside Chrome pages. | Not a foundation. A later browser extension could use them as an optional provider. |
| Dedicated grammar engine? | **Harper** (`harper-core` 3.0.0, Apache-2.0, Rust). Measured: 0.5 s startup, **2–3 ms per sentence**, fixes contractions, agreement, articles, repeated words and capitalization. It is English-only (US/UK/CA/AU/IN). | **Tier 1**: instant, offline, linked directly into the app. |
| Dedicated GEC seq2seq models (T5/CoEdIT)? | The good ones are CC BY-NC (CoEdIT, JFLEG-trained). That isn't usable commercially. | Rejected. |
| Small local LLMs? | Benchmarked Qwen3.5-0.8B, Qwen2.5-1.5B, Qwen3.5-2B and Qwen3-4B-Instruct-2507 (all Apache-2.0, Q4_K_M) on a 32-case suite. The 0.8B model changed meaning. On the suite, 1.5B scores 21/32 and 4B scores 30/32 (`bench/QUALITY.md`). | **Tier 2**, chosen automatically: Qwen3-4B (2.4 GB) with a GPU of ≥ 6 GB, otherwise Qwen2.5-1.5B (1.1 GB). |
| Runtime | llama.cpp `b12340` (MIT). The Vulkan x64 build is 36 MB, runs on NVIDIA, AMD and Intel GPUs, and falls back to CPU. It serves an OpenAI-compatible HTTP API. | Downloaded once on first use, SHA-256 pinned, bound to 127.0.0.1 only. |

## 2. Integration limits on Windows

* **UI Automation (UIA)** is the only API that works across native and web apps. Win32 Edit and RichEdit controls, WPF, WinUI, and Chromium (Chrome, Edge, Electron apps such as Discord, Teams and WhatsApp Desktop) expose `TextPattern`. Chromium switches on its accessibility tree when a UIA client queries it.
* **Where UIA fails:** apps that draw their own text (some games, terminals, Java Swing without the Access Bridge, some Qt apps), elevated (admin) windows, and UAC or secure desktops. For these Margin **shows nothing rather than guessing.**
* **Keyboard hook (`WH_KEYBOARD_LL`)** is used only to know *when* the user typed (to debounce) and to catch the popup's own shortcuts. It **does not translate or store characters**. The text is read from the field through UIA, so nothing is reconstructed from keystrokes.
* **Writing text back:** Margin presses Shift+Left N times, then sends Unicode `SendInput`, changing only the differing tail of the sentence. This works in plain edit controls, rich editors and contenteditable web editors (Lexical/Draft.js in Messenger and Facebook, Slate in Discord). It also keeps the app's own Ctrl+Z history. `ValuePattern.SetValue` was rejected because it breaks rich editors and wipes undo.
* **Clipboard fallback:** rejected. It overwrites the user's clipboard and has to steal a selection.

### Browser extension vs desktop app

| | Desktop app (UIA) | Browser extension |
|---|---|---|
| Coverage | Browsers **and** native apps | Web pages only |
| Text and caret accuracy | Good in Chromium; depends on the app's accessibility support | Exact (DOM) |
| Per-site exclusions | By window title | By URL (precise) |
| Install friction | One app | One per browser, store review |

**Decision:** a desktop app first, because one component covers the whole priority list (web apps through the browser's UIA tree, plus desktop Discord, Teams and Outlook). A browser extension is a **phase 2 companion** for exact URL exclusions and DOM-precise caret placement. It would talk to the same engine over Native Messaging.

## 3. Architecture

```
 hook thread (time-critical)     main/UI thread                 reader thread (COM MTA)         model thread
 ───────────────────────────     ──────────────────────────     ───────────────────────         ─────────────────────────
 WH_KEYBOARD_LL ──"key pressed"─▶ debounce 180 ms ──Job(id)──▶  UIA: focused field,             Provider trait
 popup shortcuts ──action──────▶ popup / tray / settings        skip password/read-only/        ├─ OpenAiCompat → llama-server
                                  ◀────────── Update(id) ─────── excluded → caret text           │   (127.0.0.1, below-normal prio)
                                                                 Harper fix (2-5 ms) ──LlmJob──▶ └─ OpenAiCompat → user's https server
                                  ◀────────── Update(id) ──────────────────────────────────────  1 predict  2 AI fix  3 rephrase
```

* **Latency:** the hook does no work beyond posting a message, so typing is never slowed down. UIA calls run off the UI thread. Each round has an id, and typing again bumps the id, so stale model output is dropped before it reaches the screen. Queued jobs are coalesced. Results are cached in memory per sentence.
* **Composition context:** the model revises the *current paragraph* (TEXT) while reading up to 800 characters of earlier paragraphs and 400 after the cursor as read-only CONTEXT. Edits are applied to the current paragraph only, so a suggestion never has to type a line break (Enter sends messages in chat apps).
* **Three writing modes,** each with its own instructions and examples: **Correct** (default: grammar, spelling and punctuation; keeps the writer's words and casual tone), **Improve** (flow and clarity, without making casual messages formal) and **Rephrase** (two alternatives with the same meaning). Settings picks the mode for automatic suggestions; Ctrl+Shift+Space shows all three.
* **Passes:** every keystroke restarts all three timers.
  * *Quick* (180 ms): Harper fixes and next-word prediction.
  * *Settle* (700 ms, or immediately after `.`, `?` or `!`): model revision; an unfinished last sentence keeps its end and gets no final punctuation.
  * *Idle* (2 s): the text is treated as a finished thought, so a final period or question mark may be suggested.
  * Rephrase runs only on finished text.
* **Rephrase styles (QuillBot-like):** Standard, Formal, Corporate (workplace tone: clear, courteous, action-oriented), Simple, Creative, Academic.
  * The full list (Ctrl+Shift+Space) has style buttons across the top. Clicking one, or Alt+←/→, redoes only the Rephrase rows in that style and saves it as the default (also in Settings).
  * Style instructions are sent with each request, so each mode's cached system prompt is reused.
  * Validation adapts per style. Only Standard must keep slang ("gonna", "rn"); Formal, Simple and Academic may write it out. Academic and Creative may replace more of the wording. Negation, thanks, apologies, names and numbers are protected in every style.
  * Measured on 10 varied sentences with Qwen3-4B: 8–9 of 10 usable per style (`bench/styles-*.md`). Screenshot: `docs/styles.png`.
* **The whole text box is checked:** every paragraph (up to 20 around the caret) gets the rule checker. The model checks the current paragraph first, then the others, each cached until it changes. Underlines and fixes work in any paragraph.
  * A paragraph is found again by its text, not by a stored offset, so fixes land correctly after the caret moves or other text changes.
  * Applying a fix elsewhere selects exactly that word (verified to still read the same), types the fix, and moves the caret back to where the user was typing.
* **Speed:** underlines stay on screen while typing and refresh as soon as a word ends (space or punctuation).
  * The model checks the current paragraph on every quick pass (120 ms after a pause, or at once after a word ends).
  * In-flight checks are **not cancelled** while the user keeps typing at the end of the paragraph: the checked text is still a prefix, so its fixes stay valid and appear mid-typing. A check of an older version never replaces a newer one.
  * Measured: underlines visible throughout continuous typing; model fixes about 0.3 s after the last key.
  * n-gram speculative decoding was tried and rejected: it was slower here (615–783 ms vs 442 ms per correction).
* **Suggestions on the text (default):**
  * No popup appears while typing. Each mistake gets a solid underline and a light highlight drawn on the word (red for correctness, blue for clarity/Improve). The highlight darkens when hovered.
  * Hovering a word opens a small pill strip right under it: the fix ("that's") and "Fix all". Click a pill, or press Tab.
  * Predicted next words appear as grey text after the caret; Tab types them.
  * Tab and Esc are captured only while a pill or a grey prediction is showing.
  * Ctrl+Shift+Space still opens the full list (Correct, Improve and Rephrase).
  * Settings → "Show suggestions on the text instead of a popup" switches back to the list popup.
  * `docs/inline.png` and `docs/prediction.png` are screenshots from the automated tests.
* **Underlines and per-fix cards (QuillBot/Grammarly style):** a correction is split into individual fixes (`text::hunks`), each labelled Spelling, Grammar, Punctuation, Capitalization or Remove. A layered, click-through overlay (`overlay.rs`) draws red squiggles under them at positions from UI Automation (`Reader::span_rects`). Hovering a squiggle opens a card with that fix and "Fix all".
  * Applying one fix: if the caret hasn't moved, the existing Shift+Left mechanism is used. If it has moved (for example, the user clicked the word), Margin selects exactly that word via `TextRange.Select` (or `EM_SETSEL` in classic edit boxes). Either way it first verifies the word still reads the same, then types the fix over it.
  * The remaining fixes reuse the same correction (no new model call).
  * Underlines follow scrolling (re-measured every 700 ms), hide while typing, and can be turned off in Settings.
* **Structured, validated output:** replies are JSON constrained by a schema, then checked by `src/validate.rs` (word alignment, protected tokens, meaning guards) before they are shown. Accepting a suggestion first re-reads the field and refuses if the text changed. Details and measurements are in `bench/QUALITY.md`.
* **Replaceable inference:** `inference::Provider` (`correct`, `rewrite`, `complete`) is the only thing the engine sees. Adding ONNX, Windows AI APIs (Phi Silica on Copilot+ PCs) or a cloud vendor means one new implementation.
* **Stack:** Rust with Win32 and GDI directly. There's no webview and no runtime to install. Tauri was rejected because a webview popup costs tens of MB per window and adds latency for a 2-line tooltip, and Rust is still needed for Harper and UIA.

## 4. Privacy and security

* Password fields (`IsPassword`), read-only fields and Margin's own windows are never read.
* Only text **before the caret in the focused field** is read (up to 600 chars), and only after a typing pause. It is never written to disk; the cache is in memory and bounded.
* Default exclusions: password managers, terminals, remote desktop. Users can add apps (`*.exe`) or window-title words (for example a bank's site name).
* No telemetry. The local model server listens on 127.0.0.1 with a random port, runs at below-normal priority, and lives inside a job object (it can't outlive Margin).
* Downloads are HTTPS-only and SHA-256 verified. The remote endpoint must use `https://`. The popup footer always shows where text goes: "On this PC" or "Sent to <host>".
* Accepting a suggestion is the only way text changes.
* **Known gap:** the optional remote API key is stored in plain text in `%APPDATA%\Margin\settings.json`. It should be encrypted with DPAPI before release.

## 5. MVP scope (this build)

Tray app with a popup at the caret (Tab to accept, Alt+Up/Down to choose, Esc to dismiss, click to accept). It offers Harper fixes, local-LLM prediction, AI fixes and rephrasing; a settings window with the eight requested options; and first-run setup. A `--probe` mode reports what Margin can read in any focused field, to check compatibility honestly.

## 6. Compatibility status

Only results from `examples/e2e.rs` or a manual `--probe` run count. Tested 2026-11-13 on Windows 11 (build 26300) with Edge.

| Target | Result | Notes |
|---|---|---|
| Classic Win32 Edit control | **Pass** (read 44 ms) | First run failed: no TextPattern. Fixed with a ValuePattern + `EM_GETSEL` fallback. |
| RichEdit control | **Pass** (read 27 ms) | TextPattern2 |
| Edge `<textarea>` | **Pass** (read 9 ms) | |
| Edge contenteditable (Gmail-like) | **Pass** (read 10 ms) | First run failed: Edge's DocumentRange spans the whole page, so **text from other fields leaked into context**. Reads are now bounded to the focused editor, and the harness checks for leaks. Also normalizes the no-break spaces Edge inserts. |
| Edge chat editor (Messenger-like, `beforeinput` model) | **Pass** (read 11 ms) | Same two fixes |
| Edge password field | **Pass** (refused) | |
| Full app: type, popup, Tab | **Pass** | Popup appeared 326 ms after the last key; focus stayed in the field; Tab applied the fix and wasn't typed into the field. |
| Underlines + hover card + single fix (RichEdit, Edge textarea, Edge chat editor) | **Pass** | Underlines 200–300 ms after the last key. A single fix applies both with the caret at the end and after clicking into the word (`e2e underline`). |
| Inline grey prediction → Tab (RichEdit), no popup shown | **Pass** | Grey text 0.5 s after the last key. Found a bug on the way: predictions were lost when queued passes were coalesced while the model was busy. |
| Fix in an earlier paragraph while typing in the next (RichEdit, Edge textarea, Edge rich editor) | **Pass** | Paragraph 1 underlined about 100 ms after typing stops in paragraph 2. Only the clicked word changes, and the caret returns to the end of paragraph 2 (`e2e paragraphs`). |
| Model server lifecycle | **Pass** | Killed when Margin is force-killed (job object). |
| Real Messenger, Facebook, Gmail, Outlook web, WhatsApp Web, Reddit | **Not tested** | Needs your sign-in; use `margin.exe --probe out.txt` |
| Discord, Teams, Outlook (desktop) | **Not tested** | Same |
| Chrome, Firefox | **Not tested** | Firefox's UIA support differs from Chromium's |

## 7. Next increments

1. Probe each priority app (signed in) and record the results in the table above.
2. Measure the CPU-only experience on a machine without a dedicated GPU, and consider a smaller model just for prediction there.
3. Encrypt the API key with DPAPI. Add "Start with Windows". Add per-app quirks if probing finds caret-rectangle issues.
4. Browser-extension companion (exact URL exclusions, DOM caret).
5. Non-English: the model tier already handles other languages. The rule tier needs Hunspell or LanguageTool for them.
