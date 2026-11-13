// Prompt variant for rephrasing an unfinished sentence without inventing an ending.
const base = `http://127.0.0.1:${process.argv[2] ?? "8091"}`;
async function chat(system, user, max = 100, temperature = 0.2) {
  const t = performance.now();
  const r = await fetch(`${base}/v1/chat/completions`, {
    method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ messages: [{ role: "system", content: system }, { role: "user", content: user }], temperature, max_tokens: max }),
  });
  const j = await r.json();
  return `${Math.round(performance.now() - t)}ms ${JSON.stringify(j.choices?.[0]?.message?.content)}`;
}
const RW_U = "You rephrase the beginning of a sentence the user is still typing. Keep the meaning, tone and person, and fix any mistakes. Rephrase only the words given: do not finish the sentence, do not add new ideas, and end at the same point. Write 2 alternatives worded differently from each other and from the original. Format:\n1. <first alternative>\n2. <second alternative>";
await chat(RW_U, "warm up", 5);
for (const s of ["I dont think thats a good idea because its", "hey just wanted to check if u got my email from", "Thanks so much for helping me with the move, I really"])
  console.log("RW-U  ", s, "=>", await chat(RW_U, s, 120, 0.7));
