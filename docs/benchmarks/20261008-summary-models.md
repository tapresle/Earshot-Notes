# Summary models against a 16 GB Mac budget

Measured October 8, 2026, for [#8](https://github.com/tapresle/Earshot-Notes/issues/8), on an Apple M5 Max with 128 GB of memory and macOS 27.0, with llama-cpp-2 0.1.158.

**Result:** every downloadable model fits the 10 GB budget at its registry context limit, so none is removed or flagged. The context limits in `src-tauri/src/models/mod.rs` are confirmed: raising them would cost memory without improving the notes.

## How to read these numbers

- **Memory and quality carry over to a 16 GB Mac.** The weights, the key-value cache, and llama.cpp's buffers take the same memory on any Apple Silicon Mac, and the same model file and prompt give effectively the same notes.
- **Speed doesn't.** These runs used an M5 Max. A 16 GB Mac usually has a base chip with far fewer GPU cores, so expect it to be several times slower. Compare speeds between models, not with what you'd see on a 16 GB Mac.
- **Memory pressure isn't measured.** Swapping and the GPU memory macOS allows only show up on a real 16 GB Mac. See [Later, with a 16 GB Mac](#later-with-a-16-gb-mac).

## Budget

The summary model may use at most **10 GB** during a call on a 16 GB Mac.

- macOS, a call app such as Zoom or Teams, a browser, Earshot Notes itself, and Whistle together need roughly 5 to 6 GB.
- Metal lets one process keep only about two-thirds of a 16 GB Mac's memory on the GPU (about 10.7 GB), so a model past that slows down sharply or fails to load.

Revise the budget when results from a real 16 GB Mac arrive.

## Method

`npm run benchmark` summarizes the two synthetic meetings in `src-tauri/tests/fixtures/meetings/` through the app's own summary code (`src-tauri/examples/summary_eval.rs`). It runs each model with its registry context limit, as the app does, at three lengths:

- **Short:** the meetings as written, about 2,000 tokens.
- **1 hour** and **2 hours:** the same meetings spread across 60 or 120 minutes of routine status updates (`--pad`), about 25,000 and 50,000 tokens.

Every transcript uses `--noisy`, which adds fillers, small talk, split sentences, and echo, as real speech recognition does. Each meeting plants specifics (ticket numbers, error messages, amounts, corrected values, owners, and conditions), 25 per length, and the benchmark counts how many survive into the notes.

Each run is its own process under `/usr/bin/time -l`. **Peak memory** is the maximum resident set size, which counts the memory-mapped weights and Metal's buffers. macOS's "peak memory footprint" leaves out the mapped weights (0.47 GB for the 3.1 GB Gemma 4 E2B), so it's recorded but not compared with the budget.

llama.cpp creates each context at the prompt's length plus the room for the notes, not at the model's limit, so peak memory grows with the meeting. A straight-line fit of peak memory against the context used separates fixed memory from memory per token, and estimates the peak for a meeting that fills the limit.

The raw results, including every summary, are in `20261008-summary-models.jsonl`. To rebuild a table, run `npm run benchmark -- --report <file>`.

## Results at the registry limits

| Model | Context limit | Peak memory (short / 1 h / 2 h) | Per 1K tokens of context | Peak at the limit (estimated) | Within 10 GB | Writing speed | First text, 2 h meeting | Specifics kept |
|---|---|---|---|---|---|---|---|---|
| Gemma 4 E2B | 32,768 | 3.2 / 3.7 / 3.8 GB | 23 MB | 3.8 GB | Yes | 136 tokens/s | 37 s | 49 of 75 |
| Phi-4 mini | 32,768 | 3.2 / 5.9 / 6.5 GB | 130 MB | 6.5 GB | Yes | 84 tokens/s | 46 s | 53 of 75 |
| Qwen3 4B Instruct | 32,768 | 3.4 / 6.6 / 7.0 GB | 146 MB | 7.0 GB | Yes | 94 tokens/s | 67 s | 65 of 75 |
| Gemma 4 E4B | 32,768 | 5.2 / 6.6 / 6.7 GB | 61 MB | 6.7 GB | Yes | 63 tokens/s | 61 s | 63 of 75 |
| Qwen3 8B | 16,384 | 5.7 / 7.1 / 7.1 GB | 146 MB | 7.1 GB | Yes | 65 tokens/s | 97 s | 66 of 75 |
| Ministral 3 8B | 16,384 | 5.9 / 7.1 / 7.1 GB | 137 MB | 7.1 GB | Yes | 62 tokens/s | 93 s | 70 of 75 |
| Ministral 3 14B | 8,192 | 8.4 / 9.1 / 9.1 GB | 171 MB | 9.1 GB | Yes | 42 tokens/s | 176 s | 70 of 75 |

- **Writing speed** is output tokens per second after the first text appears, averaged over all six runs.
- **First text** runs from the start of the summary to the first words of the notes. For a long meeting, it includes reading the transcript and condensing it in parts.
- **Specifics kept** includes "Corvina", which the noisy transcript spells "Corvena". It tests the vocabulary, and the benchmark passes no terms, so most models miss it every time: 72 of 75 is the realistic maximum.

## Context limits

The three models with lower limits also ran at the app-wide 32K cap (`20261008-summary-models-at-32k.jsonl`):

| Model | At its limit | At 32K |
|---|---|---|
| Qwen3 8B (16K) | 7.1 GB, 66 of 75 | 9.4 GB, 66 of 75 |
| Ministral 3 8B (16K) | 7.1 GB, 70 of 75 | 9.3 GB, 71 of 75 |
| Ministral 3 14B (8K) | 9.1 GB, 70 of 75 | 12.9 GB, over budget, 64 of 75 |

- **The 8B models stay at 16K.** At 32K they fit the budget by only 0.6 GB and keep the same specifics.
- **Ministral 3 14B stays at 8K.** At 32K it's over budget, and it keeps fewer specifics. It's trained on 16K and stretched to 256K with YaRN, so a longer context reads worse.
- **The per-token figures match the architectures.** Gemma 4 attends to only a short window in most layers, so its cache grows two to six times slower than the others'. That's why E2B's peak barely moves from a short meeting to a two-hour one.

Scaling the limits with the Mac's memory, and measuring quality past 32K, is [#93](https://github.com/tapresle/Earshot-Notes/issues/93).

## Quality

From the saved notes for each run:

- **Ministral 3 8B** writes the most complete notes. It keeps causes, results, and conditions ("Release 4.2 cut the client timeout from 10 s to 5 s") and holds up best in long meetings. It's heavy on bold text.
- **Ministral 3 14B** keeps as much as the 8B model, at nearly twice the time and 2 GB more memory. Its 8K limit means a two-hour meeting is condensed in seven or eight parts, so a 16 GB Mac would take minutes to start writing.
- **Qwen3 4B Instruct** is the strongest small model: detailed notes with numbered next steps, and the fastest writing apart from Gemma 4 E2B.
- **Qwen3 8B** is detailed, but sometimes names "Meeting Participants" as a task's owner instead of leaving the owner out.
- **Gemma 4 E4B** keeps nearly everything in short and one-hour meetings, with well-structured cause-and-effect notes, but lost most of the office move in one two-hour meeting (4 of 12).
- **Gemma 4 E2B** writes good notes for short meetings, but loses specifics as meetings grow. In one two-hour meeting it summarized only the status updates and kept none of the planted specifics. Whether it should stay the recommended model is [#96](https://github.com/tapresle/Earshot-Notes/issues/96).
- **Phi-4 mini** writes the shortest notes and drops causes and error messages even in short meetings.

### Long meetings lose the specifics

Most models kept fewer specifics in the two-hour meetings, where the transcript is condensed in parts; only Ministral 3 8B and 14B held steady. The padding here is extreme, more than 90% routine status updates, but it shows a real risk: when condensing, a smaller model can keep the repetitive material and drop the few minutes that mattered. That's [#95](https://github.com/tapresle/Earshot-Notes/issues/95).

## Later, with a 16 GB Mac

Run `npm run benchmark` on a real 16 GB Mac, from a tester or a briefly rented cloud Mac. The results append to a new JSONL file with that Mac's details. Use them to confirm the budget, and to measure real speed and memory pressure.
