// Model benchmark: run correction / rewrite / completion prompts against a
// running llama-server and print outputs with latency.
// Usage: node bench/bench.mjs <port> <label>
const port = process.argv[2] ?? "8091";
const label = process.argv[3] ?? "model";
const base = `http://127.0.0.1:${port}`;

const SYS_FIX =
  "You fix spelling, grammar and punctuation. Reply with the corrected text only. Keep the meaning, tone and wording; change as little as possible. If the text is already correct, repeat it unchanged.";
const SYS_REWRITE =
  "You rewrite text so it reads naturally and clearly. Keep the meaning and the tone. Give exactly 2 alternative versions, one per line, no numbering, no quotes, no commentary.";

const fixCases = [
  "I dont think thats a good idea because its",
  "their going to the store tomorow and i will meet them their.",
  "me and him was going to the party but it got cancel",
  "Can you sent me the file when your free?",
  "The meeting is at 3pm tomorrow.",
];
const rewriteCases = [
  "I dont think thats a good idea because its too expensive",
  "hey just wanted to check if u got my email from yesterday about the thing",
];
const completeCases = [
  "Thanks for getting back to me so",
  "I'm sorry I couldn't make it to the meeting, I",
  "Let me know if you have any",
];

async function chat(system, user, max = 80) {
  const t = performance.now();
  const r = await fetch(`${base}/v1/chat/completions`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      messages: [{ role: "system", content: system }, { role: "user", content: user }],
      temperature: 0.2, max_tokens: max,
      chat_template_kwargs: { enable_thinking: false },
    }),
  });
  const j = await r.json();
  return { ms: Math.round(performance.now() - t), out: j.choices?.[0]?.message?.content?.trim() ?? JSON.stringify(j) };
}

async function complete(prefix) {
  const t = performance.now();
  const r = await fetch(`${base}/completion`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ prompt: prefix, n_predict: 8, temperature: 0, stop: ["\n", ".", "!", "?"], cache_prompt: true }),
  });
  const j = await r.json();
  return { ms: Math.round(performance.now() - t), out: j.content };
}

console.log(`=== ${label} ===`);
await chat(SYS_FIX, "warm up"); // warm-up
for (const c of fixCases) { const r = await chat(SYS_FIX, c); console.log(`FIX  ${r.ms}ms\n  in : ${c}\n  out: ${r.out}`); }
for (const c of rewriteCases) { const r = await chat(SYS_REWRITE, c, 120); console.log(`REWRITE ${r.ms}ms\n  in : ${c}\n  out: ${r.out.replace(/\n/g, "\n       ")}`); }
for (const c of completeCases) { const r = await complete(c); console.log(`NEXT ${r.ms}ms  ${c}|${r.out}`); }
