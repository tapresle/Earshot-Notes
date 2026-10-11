# Qwen3.5 4B as the recommended local AI model

- **Status:** Accepted
- **Date:** 2026-10-11

## Context

Gemma 4 E2B became the recommended model in [`20261003-embedded-llama-cpp.md`](20261003-embedded-llama-cpp.md) because it was small and fast. The October 8 benchmark showed that it loses specifics as meetings grow, and in one two-hour meeting it kept none. [#96](https://github.com/tapresle/Earshot-Notes/issues/96) asked whether it should stay the recommended model, and whether the list should change.

The recommended model has to fit the 10 GB budget on a 16 GB Mac, keep the specifics in long meetings, start writing soon, and answer questions in chat. Its license must be Apache 2.0 or MIT.

## Decision

Recommend Qwen3.5 4B (`Qwen3.5-4B-Q4_K_M.gguf`, 2.7 GB, Apache 2.0). Offer four models in all:

- **Qwen3.5 4B,** recommended.
- **Gemma 4 E4B,** from its QAT file (`gemma-4-E4B-it-qat-UD-Q4_K_XL.gguf`) instead of Q4_K_M.
- **Ministral 3 8B,** unchanged.
- **Qwen3.5 9B,** new.

Remove Gemma 4 E2B, Phi-4 mini, Qwen3 4B Instruct, Qwen3 8B, and Ministral 3 14B.

## Evidence

From [`docs/benchmarks/20261011-summary-models.md`](../benchmarks/20261011-summary-models.md):

- Qwen3.5 4B kept 73 of 75 specifics, tied for the most, against Gemma 4 E2B's 59.
- Its peak memory is 3.8 GB in a two-hour meeting, the same as Gemma 4 E2B. Its cache grows 34 MB per 1K tokens, against 147 MB for Qwen3 4B Instruct.
- It started writing a two-hour meeting's notes in 28 seconds on average, the soonest of any model.
- It writes as fast as the other small models, at 111 tokens per second.
- Qwen3.5 9B matched its specifics with no mistakes, in 6.6 GB.

## Alternatives considered

- **Keep Gemma 4 E2B, or switch to its QAT file:** the QAT file uses less memory, but both keep 59 or 60 specifics.
- **Ministral 3 8B as the recommended model:** it keeps as many specifics, but needs 7.1 GB, a 16K context, and twice as long to start writing.
- **Qwen3.5 9B as the recommended model:** the most accurate notes, but a 5.7 GB download and 6.6 GB of memory. It stays on the list for people who want it.
- **Granite 4.1:** the 3B model kept the fewest specifics in long meetings, and the 8B model is over the budget.
- **Gemma 4 12B QAT:** its cache grows 341 MB per 1K tokens, so it reaches 17.1 GB in a two-hour meeting.

## Consequences

- Qwen3.5 is a thinking model. The app renders its chat template with thinking turned off, as it already did for Qwen3, so the notes contain no reasoning text.
- Every listed model now runs at the 32K context limit except Ministral 3 8B, which stays at 16K.
- Anyone who downloaded a removed model keeps it as an installed model. It no longer has a registry entry, so it runs at the app-wide 32K context limit instead of its former limit. For Ministral 3 14B, that exceeds the memory budget. Removing it from **Settings** and downloading a listed model avoids that.
- The README and the website list the new models.
