# Transcription models: Whistle and Whisper

Measured October 8 and 9, 2026, for [#25](https://github.com/tapresle/Earshot-Notes/issues/25), on an Apple M5 Max with 128 GB of memory and macOS 27.0, with whisper.cpp `v1.9.5` and needle engine 3.1.

**Result:** Whisper large-v3-turbo makes about half of Whistle's word errors, recognizes far more technical terms (18 of 26 against 7 at phone quality), and runs comfortably on Apple Silicon. Earshot offers it as an optional download, and Whistle stays the bundled default. See [`docs/decisions/20261009-whisper-turbo-option.md`](../decisions/20261009-whisper-turbo-option.md).

## What was compared

| Engine | Model | File | How it runs |
|---|---|---|---|
| needle | Whistle (bundled today) | 17 MB | Metal |
| whisper.cpp | Whisper small, `ggml-small-q5_1.bin` | 190 MB | Metal, or CPU only |
| whisper.cpp | Whisper large-v3-turbo, `ggml-large-v3-turbo-q5_0.bin` | 574 MB | Metal, or CPU only |

CPU-only runs stand in for Windows and Linux machines without a usable GPU.

## Synthetic speech with known text

30 invented meeting sentences (in the appendix), 20 of them technical, spoken by four macOS voices with `say`. Each was transcribed whole, clean and filtered to phone quality (300 Hz to 3.4 kHz, eighth-order), which is what the other side of a call sounds like. Word error rate counts substitutions, insertions, and deletions after normalizing case, punctuation, and how numbers are written ("10%" and "ten percent" match).

| Engine | Word errors, clean | Word errors, phone quality | Technical terms right, clean | Technical terms right, phone quality |
|---|---|---|---|---|
| Whistle | 23.1% | 26.6% | 11 of 26 | 7 of 26 |
| Whisper small | 17.6% | 19.4% | 17 of 26 | 14 of 26 |
| **Whisper large-v3-turbo** | **10.9%** | **12.8%** | **20 of 26** | **18 of 26** |
| Turbo, with the #104 software terms as its prompt | 11.4% | 14.1% | 20 of 26 | 19 of 26 |

- Whistle turned "Kubernetes" into "Cuban needs", "OAuth" into "wild", and "async" into nonsense, and lost whole technical sentences at phone quality.
- Turbo's remaining errors fall on the same kind of jargon, and on a few sentences in one synthetic voice that every engine garbled.
- Whisper has no keyword biasing like needle's, only an initial prompt. Giving it the vocabulary as a prompt barely helped the terms (one more right at phone quality) and cost overall accuracy, so Earshot doesn't send it.

## A real two-hour call

A 2 hour 20 minute release call, recorded in two parts with **Keep audio recordings**, analyzed locally. The recording isn't shared, and nothing from it is quoted here. Both sources went through the app's chunker (2-second pauses, #99), giving 612 clips and 63 minutes of speech, and every engine transcribed the same clips.

- **Accuracy:** on the other participants' side (call audio), turbo transcribed technical phrases and whole questions correctly that Whistle mangled or dropped. Whisper small landed between them, closer to turbo.
- **Invented phrases on faint clips:** the app's phantom filter (`transcribe/phantom.rs`) dropped 68 lines such as "Thank you." from Whistle, and 68 from turbo (57 of them the same clips). Whisper small instead wrote sound labels, such as `[BLANK_AUDIO]` and `(clicking)`, on most of the 107 faint clips. A Whisper integration strips bracketed and parenthesized labels.
- **Technical terms:** the call wasn't very technical. Of the #104 preset terms, turbo transcribed "null", "API", "payload", and "SQL"; Whistle transcribed only "API".

## Speed and memory

| Engine | Metal: 612 clips, 63 min of speech | CPU only, 8 threads: 100 clips, 11.9 min | Peak memory, Metal | Peak memory, CPU |
|---|---|---|---|---|
| Whistle | 14 s | 2.6 s | small | small |
| Whisper small | 60 s, about 63 times real time | 54 s, about 13 times real time | 0.5 GB | 0.55 GB |
| Whisper large-v3-turbo | 96 s, about 39 times real time | 186 s, about 4 times real time | 0.82 GB | 0.89 GB |

Times include loading the model once per 100 clips. Speed comes from an M5 Max and is faster than on a base chip; on a slower CPU, turbo may not keep up with a live call, which matters for Windows and Linux.

## What each model offers

The source for how the README, the website, and the app describe the choice.

**Whistle, included with Earshot Notes**
- Works from the first launch, with nothing to download.
- Tiny (17 MB) and very fast: a fraction of a second per clip, and little memory or battery.
- Listens for your **Vocabulary** terms as it transcribes.
- Best with clear audio, such as your own microphone or a call with good audio.

**Whisper large-v3-turbo, an optional 574 MB download**
- About half the transcription errors of Whistle, on clean and call audio alike.
- Recognizes far more technical terms, such as API names and infrastructure words: 18 of 26 at phone quality, against Whistle's 7.
- Much better on the other side of a call, where call apps reduce the audio to phone quality.
- Still fast on Apple Silicon (about 40 times real time) and uses under 1 GB of memory.
- Your **Vocabulary**'s **Heard as** corrections and summary spellings still apply, but it doesn't listen for terms the way Whistle does.

**Wording in the app**
- **Settings** > **Transcription**, Whistle: "Fast and light, and works from the first launch."
- **Settings** > **Transcription**, Whisper large-v3-turbo: "More accurate, especially for the other side of calls and technical terms."
- Under a finished transcript, until Whisper is chosen or the suggestion is dismissed: "For a more accurate transcript, especially of the other people on a call, download Whisper large-v3-turbo (574 MB) in Settings."

## Reproducing

1. Build whisper.cpp `v1.9.5` with CMake (Metal is the default on macOS), and download the two models from `huggingface.co/ggerganov/whisper.cpp`, checking each file's SHA-256.
2. Generate the sentences below with `say -o <file> --data-format=LEI16@16000 --channels=1`, rotating the voices Samantha, Daniel, Karen, and Moira, and filter copies to 300 Hz to 3.4 kHz.
3. Transcribe each file whole with Whistle and with `whisper-cli -l en -nt -oj`, and score against the sentences.

## Appendix: the synthetic sentences

1. The PATCH endpoint returns a four hundred error when the request body is null.
2. We should make that field nullable and add a migration before Friday.
3. Can you check the query params on the search API and send me the payload?
4. The webhook retries three times and then writes the event to the dead letter queue.
5. Let's refactor the async handler so it awaits the database call.
6. The deploy to staging failed because the Kubernetes pod ran out of memory.
7. I pushed the fix to GitHub and opened a pull request for review.
8. The JSON schema says the amount is an integer, but the client sends a string.
9. We rotated the OAuth token, so the integration needs to sign in again.
10. Turn on the feature flag for ten percent of users on Monday morning.
11. The cron job runs at two in the morning and exports yesterday's invoices.
12. Add a regex to strip the dashes from the phone number before we save it.
13. Our test coverage dropped to seventy two percent after the last merge.
14. The middleware logs every request, which is why the disk filled up.
15. Rollback the release if the error rate goes above one percent.
16. We can cache the response for five minutes to cut the latency in half.
17. The TypeScript build fails because the enum value isn't exported.
18. Run the linter locally before you push, it saves a round of review.
19. The SQL query is slow because it scans the whole orders table.
20. I'll write the runbook and share it in the channel this afternoon.
21. Thanks everyone, let's pick this back up on Thursday at ten.
22. Sorry, you cut out for a second, could you repeat the last part?
23. The budget for next quarter is forty eight thousand dollars.
24. Priya will own the vendor contract and send it to legal by Wednesday.
25. I think the second option is cheaper, but the first one is faster.
26. We moved the offsite from March to April because of the weather.
27. The customer asked whether they can export their data as a spreadsheet.
28. Please send the slides before the meeting so people can read ahead.
29. Marcus is out next week, so Dana will cover the support queue.
30. Let's keep this short, we only have fifteen minutes left.
