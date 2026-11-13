# Model benchmark (2026-11-13)

Machine: Intel i7-13620H (10 cores), 16 GB RAM, RTX 4050 Laptop 6 GB. Runtime: llama.cpp b12340, Vulkan build. Models are Q4_K_M; the CPU runs used 8 threads. Script: `bench/bench.mjs`.

## Correction quality

| Input | Qwen3.5-0.8B | Qwen2.5-1.5B |
|---|---|---|
| I dont think thats a good idea because its | unchanged (missed both) | I don't think that's a good idea because it is. |
| their going to the store tomorow and i will meet them their. | They **will go**… (meaning changed) | They are going to the store tomorrow, and I will meet them there. |
| me and him was going to the party but it got cancel | only "cancelled" | Me and him were going to the party, but it got canceled. |
| Can you sent me the file when your free? | CPU correct; GPU "when you free" | Can you send me the file when you're free? |
| The meeting is at 3pm tomorrow. (correct) | unchanged (CPU) / 3:00pm (GPU) | 3 pm / 3:00 pm (style nit) |

**Decision:** Qwen2.5-1.5B is the only local model shipped. The 0.8B model is rejected because it missed errors and changed meaning.

## Latency (Qwen2.5-1.5B, after warm-up)

| | CPU only | GPU (Vulkan) |
|---|---|---|
| Correction (one sentence) | 310–510 ms | 140–180 ms |
| Two rephrasings | 550–950 ms | 190–220 ms |
| Next-words prediction | 90–240 ms | 35–75 ms |
| First request (cold) | — | ~1.3 s, so the app warms up at start |
| Server memory (RSS) | ~1.2 GB | ~1.1 GB |

Harper (the rule tier): 2–3 ms per sentence, about 0.5 s one-time startup.

## Prompt findings

* An instruction telling the model not to complete sentences made it **worse**: it invented "because it seems risky" and stopped fixing "me and him was". Margin keeps the plain prompt and post-processes instead (`text::trim_invented_ending`).
* Asking for one alternative per line returned one line. The numbered format (`1. … 2. …`) reliably returns two distinct alternatives.
* For rephrasing an *unfinished* sentence, every prompt tried invented an ending. Margin therefore rephrases only finished sentences.
* Raw completion sometimes returns fill-in-the-blank artifacts (`_______ my car`), which are filtered out.
