# Write summaries in one pass, not two

- **Status:** Accepted
- **Date:** 2026-10-16

## Context

The notes come from one model call over the transcript (`summary_messages`), or over its condensed parts when the meeting is too long (`combine_messages`). Twice, a gap in the notes has suggested adding a first pass that prepares material for the writer:

- [#70](https://github.com/tapresle/Earshot-Notes/issues/70) tried **extract, then write.** The first pass lists the facts in the transcript, and the writer gets that list along with the transcript. This is the `--approach extract` option in `summary_eval`.
- [#162](https://github.com/tapresle/Earshot-Notes/issues/162) tried **outline, then write.** The first pass plans the notes: the topics in order, the note-taker's points the meeting never discussed, the personal details, and the tasks. The writer gets that plan along with the same input. This was its named fallback after the prompt changes in [#164](https://github.com/tapresle/Earshot-Notes/pull/164).

## Decision

Keep one pass. Don't add a planning or extraction pass in front of the notes to recover specifics, sections, or the note-taker's points. Fix those gaps in the one-pass prompt, or with a larger model.

## Evidence

### Extract, then write (#70, 2026-10-08)

Measured on Gemma 4 E4B, with 25 specifics planted across two meetings ([results](https://github.com/tapresle/Earshot-Notes/issues/70#issuecomment-6050430141)):

- It took twice as long: 73 + 74 seconds on the long meeting, against 40 + 32 for one pass.
- It kept 20 specifics on the long meeting, against 24 for one pass.
- It brought back values that speakers had corrected. The extraction lists both values, and the writer can't tell which one stands.

### Outline, then write (#162, 2026-10-16)

Measured on Qwen3.5 4B with `summary_eval` on all three fixtures. Each fixture ran as a single pass and condensed (`--context 8192 --pad 20|30|40`), each clean and with `--noisy`: 24 runs per prompt. Three wordings of the plan were measured against `main` at `f22ef90`:

| Metric | One pass | Plan v2 | Plan v3 | Plan v4 |
|---|---|---|---|---|
| Specifics kept (of 352) | 337 | 335 | 332 | 331 |
| `## Next steps` present (of 24) | 13 | 21 | 22 | 23 |
| Undiscussed hiring point, exact (of 8) | 0 | 0 | 3 | 1 |
| Porto under `## Personal notes` (of 8) | 3 | 4 | 3 | 4 |
| `## Personal notes` in billing-incident or office-move (of 16) | 1 | 3 | 4 | 6 |
| Runs with a padding name as a task owner | 2 | 4 | 4 | 3 |
| "You" plus a third-person verb, such as "You signs" | 0 | 4 | 4 | 1 |

No version invented ticket numbers or copied a prompt example. The plan made each summary about 1.2 to 1.5 times as long to write.

What the notes showed:

- **The plan was right, and the writer ignored it.** In single passes, the plan listed "ask about hiring plan for Q1 (not discussed)". The notes still dropped it, or turned it into a heading as if the meeting had discussed it. In condensed runs, the plan also marked topics the meeting did discuss as not discussed.
- **The plan loses what condensing kept.** The condensed parts kept Porto (`found_in_parts` was 18 of 19, missing only the hiring point), but most plans built from them said there was nothing personal.
- **Asking for a list invites filler.** With a Personal list to fill, the plan listed the noisy transcript's weather remark, and the writer put it under `## Personal notes` in work meetings. Filtering it out would mean telling the model to drop small talk, which the summaries keep at every stage.
- **The plan adds its own errors.** Names from the padding's status updates became task owners, and "Me writes" in the plan became "You writes" in the notes.

## Alternatives considered

- **Parse the plan and add its undiscussed points to the notes in code:** in condensed runs the plan lists discussed topics as undiscussed, so code would add false lines.
- **More wordings of the plan:** three wordings moved the counts by one or two in either direction. None fixed the hiring point or Porto, and each one put personal notes into work meetings more often than one pass did.

## Consequences

- One summary takes one model call, or one per condensed part plus one.
- The one real gain from the plan was `## Next steps` (13 to as many as 23 of 24 runs, and 2 to 6 of 6 single passes). That probably comes from handing the writer an explicit task list, not from the extra call, so it's worth trying as a one-pass prompt change.
- `summary_eval` uses a fixed sampling seed (`LlamaSampler::dist(1234)` in `src/llm/llama.rs`), so repeating a run gives identical notes. To measure a prompt change, compare more configurations or fixtures, not repeated runs.
