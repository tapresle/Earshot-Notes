# Whisper large-v3-turbo as an optional transcription model

- **Status:** Accepted
- **Date:** 2026-10-09

## Context

Whistle, bundled with Earshot Notes through the needle engine, is tiny and fast but makes many errors on technical terms and on call audio, which call apps reduce to phone quality. #25 looked for a more accurate model that still runs on the device, under Earshot's GPL 3.0 license, and on Windows and Linux as well as macOS, which Earshot may support later.

## Decision

Offer Whisper large-v3-turbo (`ggml-large-v3-turbo-q5_0.bin`, 547 MB) as an optional download, run through whisper.cpp built from source at a pinned release. Whistle stays bundled and the default, so transcription works from the first launch.

## Evidence

From [`docs/benchmarks/20261009-transcription-models.md`](../benchmarks/20261009-transcription-models.md):

- Word errors on synthetic speech: 10.9% clean and 12.8% at phone quality, against Whistle's 23.1% and 26.6%.
- Technical terms right at phone quality: 18 of 26, against Whistle's 7.
- On a real two-hour call, turbo transcribed technical phrases and questions on the participants' side that Whistle mangled.
- About 39 times real time on Metal on an M5 Max, and under 1 GB of memory.

## Alternatives considered

- **Cactus's main engine (Whisper, Moonshine, and Parakeet as Cactus publishes them):** its license limits use to individuals, small organizations, education, and non-profits, so it can't ship in GPL-3.0-only Earshot. Only Whistle runs on needle.
- **FluidAudio (Parakeet on Apple's Neural Engine):** fast and Apache 2.0, but Apple platforms only. Windows and Linux would need a second engine.
- **sherpa-onnx (Parakeet, Whisper, and more):** cross-platform and Apache 2.0, but ships ONNX Runtime, a large dependency. Kept in reserve if Whisper had disappointed.
- **Whisper small:** 181 MB and about 3.5 times faster on the CPU, but makes noticeably more errors (17.6% and 19.4%). Not offered on the Mac; revisit as the fallback for Windows and Linux machines whose CPUs can't keep up with turbo.
- **Whisper as the default:** more accurate, but a 547 MB download would stand between installing the app and the first recording. The app suggests it instead.

## Consequences

- whisper.cpp (MIT) becomes a native dependency, built from source per platform, with a small C interface and a binding like `needle-sys`. Its license and Whisper's (MIT) join the bundled notices.
- whisper.cpp and llama.cpp each include ggml, and two copies can't be linked into one binary. whisper.cpp is built into its own dynamic library that exports only Earshot's C interface, bundled in the app and opened when Whisper is first used. The library adds about 6.6 MB to the app.
- The model is downloaded, pinned, and verified like the summary models, and chosen in **Settings**.
- Whisper has no keyword biasing, so the vocabulary's terms aren't sent to it; a prompt of terms measured slightly worse. **Heard as** corrections and summary spellings still apply.
- Whisper writes sound labels such as `[BLANK_AUDIO]` on faint clips, which Earshot strips, and invents "Thank you." like Whistle, which the existing phantom filter catches.
- The README, the website, and the app describe both models from the highlights in the benchmark write-up, and the app suggests the download for more accurate transcripts.
