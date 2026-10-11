# Choosing the recommended local AI model

Measured October 10, 2026, for [#96](https://github.com/tapresle/Earshot-Notes/issues/96), on an Apple M5 Max with 128 GB of memory and macOS 27.0, with llama-cpp-2 0.1.158.

**Result:** Qwen3.5 4B replaces Gemma 4 E2B as the recommended model. It ties for the most specifics kept, uses no more memory than Gemma 4 E2B, and starts writing a two-hour meeting's notes the soonest. The list becomes Qwen3.5 4B, Gemma 4 E4B (its QAT file), Ministral 3 8B, and Qwen3.5 9B. The decision is in [`20261011-recommended-summary-model.md`](../decisions/20261011-recommended-summary-model.md).

## Method

The method, the budget, and the caveats are the same as in [`20261008-summary-models.md`](20261008-summary-models.md):

- `npm run benchmark` summarizes the two synthetic meetings with `--noisy`, short and padded to 1 hour and 2 hours, at each model's registry context limit.
- Each length plants 25 specifics, and the benchmark counts how many survive into the notes.
- The budget is 10 GB of peak memory during a call on a 16 GB Mac.
- Memory and quality carry over to a 16 GB Mac. Speed doesn't, so compare speeds between models only.

Every candidate, including the models already listed, ran again in one session, so the speeds compare with each other but not with the October 8 runs.

The raw results, including every summary, are in `20261011-summary-models.jsonl`. To rebuild the table, run `npm run benchmark -- --report docs/benchmarks/20261011-summary-models.jsonl`.

## Candidates

- **The seven listed models,** except Phi-4 mini, which kept the fewest specifics on October 8.
- **Qwen3.5 4B and 9B:** the successor to Qwen3. Most layers use Gated DeltaNet, a linear attention whose memory doesn't grow with the meeting, so their key-value cache grows about four times slower than Qwen3's.
- **QAT files for Gemma 4 E2B, E4B, and 12B:** Google trained these to survive 4-bit quantization. Unsloth's UD-Q4_K_XL builds are smaller than the Q4_K_M files.
- **Granite 4.1 3B and 8B:** IBM's Apache 2.0 models, trained for enterprise summarization and retrieval.

Models that were considered and not run:

- **LFM2.5 and Nemotron Nano:** their licenses add terms beyond Apache 2.0 and MIT.
- **Models fine-tuned only for summarization:** most are non-commercial, and none also answers questions, which chat needs.
- **Models the bundled llama.cpp can't load:** newer architectures wait for a llama-cpp-2 upgrade.

## Results

| Model | Context limit | Peak memory (short / 1 h / 2 h) | Per 1K tokens of context | Within 10 GB | Writing speed | First text (2 h) | Specifics kept | Download |
|---|---|---|---|---|---|---|---|---|
| Gemma 4 E2B | 32,768 | 3.2 / 3.7 / 3.8 GB | 23 MB | Yes | 105 tokens/s | 33 s | 59 of 75 | 3.1 GB |
| Gemma 4 E2B QAT | 32,768 | 2.8 / 3.3 / 3.4 GB | 22 MB | Yes | 119 tokens/s | 37 s | 60 of 75 | |
| Granite 4.1 3B | 32,768 | 2.6 / 4.2 / 4.6 GB | 82 MB | Yes | 106 tokens/s | 42 s | 52 of 75 | |
| Qwen3 4B Instruct | 32,768 | 3.3 / 6.6 / 7.0 GB | 147 MB | Yes | 110 tokens/s | 59 s | 69 of 75 | |
| **Qwen3.5 4B** | 32,768 | 3.0 / 3.8 / 3.8 GB | 34 MB | Yes | 111 tokens/s | 28 s | **73 of 75** | 2.7 GB |
| Gemma 4 E4B | 32,768 | 5.2 / 6.6 / 6.7 GB | 62 MB | Yes | 93 tokens/s | 41 s | 68 of 75 | |
| **Gemma 4 E4B QAT** | 32,768 | 4.5 / 5.9 / 6.0 GB | 61 MB | Yes | 99 tokens/s | 31 s | 70 of 75 | 4.2 GB |
| Qwen3 8B | 16,384 | 5.7 / 7.1 / 7.1 GB | 149 MB | Yes | 86 tokens/s | 53 s | 71 of 75 | |
| **Ministral 3 8B** | 16,384 | 5.8 / 7.1 / 7.1 GB | 141 MB | Yes | 80 tokens/s | 55 s | 73 of 75 | 5.2 GB |
| Granite 4.1 8B | 32,768 | 6.1 / 9.4 / 10.2 GB | 162 MB | No | 68 tokens/s | 74 s | 70 of 75 | |
| **Qwen3.5 9B** | 32,768 | 5.7 / 6.5 / 6.6 GB | 34 MB | Yes | 74 tokens/s | 39 s | 73 of 75 | 5.7 GB |
| Gemma 4 12B QAT | 32,768 | 8.6 / 16.2 / 17.1 GB | 341 MB | No | 54 tokens/s | 76 s | 71 of 75 | |
| Ministral 3 14B | 8,192 | 8.4 / 9.1 / 9.1 GB | 171 MB | Yes | 55 tokens/s | 98 s | 70 of 75 | |

Bold rows are the models the app now lists.

- **Writing speed** is output tokens per second after the first text appears, averaged over all six runs.
- **First text (2 h)** is the average time from the start of a two-hour meeting's summary to the first words of the notes. It includes reading the transcript and condensing it in parts.

## Quality

From the saved notes for each run:

- **Qwen3.5 4B** writes detailed notes that hold up in two-hour meetings, where the older small models lost specifics. Five of its six runs were accurate. In the short billing meeting it made three mistakes: it named the wrong person as the email's author, described the retry as a second valid charge, and left out your task to write an alert.
- **Qwen3.5 9B** was accurate in all six runs, with the same detail as the 4B model.
- **Ministral 3 8B** is still the most complete on causes and conditions, at the slowest speed of the listed models.
- **Gemma 4 E4B QAT** keeps more specifics than the Q4_K_M file, in less memory. In two-hour meetings it also lists every routine status update, which makes the notes long, and it sometimes leaves a task's owner unspecified when the transcript names one.
- **Gemma 4 E2B** and its QAT file still lose specifics as meetings grow.
- **Granite 4.1 3B** kept the fewest specifics in two-hour meetings (10 of 25).

Neither Qwen3.5 model wrote any thinking text. The app renders their chat template with thinking turned off, which a template test now covers.

## What changed in the list

| Model | Change | Why |
|---|---|---|
| Qwen3.5 4B | Added, recommended | Most specifics kept, Gemma 4 E2B's memory, soonest first text |
| Qwen3.5 9B | Added | As complete as Ministral 3 8B, with 0.5 GB less memory and a 32K context |
| Gemma 4 E4B | QAT file replaces Q4_K_M | More specifics, 0.7 GB less memory, a smaller download |
| Ministral 3 8B | Kept | Most complete notes on causes and conditions |
| Gemma 4 E2B | Removed | Qwen3.5 4B keeps 14 more specifics in the same memory |
| Phi-4 mini | Removed | Fewest specifics on October 8 |
| Qwen3 4B Instruct | Removed | Qwen3.5 4B keeps more in about half the memory |
| Qwen3 8B | Removed | Qwen3.5 9B keeps more, with less memory and twice the context |
| Ministral 3 14B | Removed | No better than Ministral 3 8B, at twice the time to first text |
