# Capture microphone and system audio as separate streams

- **Status:** Accepted
- **Date:** 2026-10-03

## Context

Commercial meeting note apps transcribe both sides of a call with no bot in the meeting. Whistle has no speaker diarization, and it handles at most 30 seconds of 16 kHz mono audio per pass.

## Decision

Capture the microphone and the system audio output as two independent streams. Label microphone segments "Me" and system audio segments "Them". Each stream is resampled to 16 kHz mono and split on silence into segments of 28 seconds or less.

- **macOS:** Core Audio process taps (API available from macOS 14.2; the app requires macOS 26 because of the needle engine), reached through a small Objective-C static library with a C ABI in `src-tauri/crates/macos-audio`. Objective-C replaced the originally planned Swift because every API involved is C or Objective-C, and linking a Swift static library into Rust requires wiring up the Swift runtime by hand.
- **Windows (later):** WASAPI loopback through `cpal`.
- **Linux (later):** a PulseAudio or PipeWire monitor source.

## Alternatives considered

- **Microphone only:** This is simpler. It misses the other side whenever you wear headphones, and it can't tell speakers apart.
- **A diarization model:** This adds a second model and its latency, and it still only produces anonymous speaker labels.

## Consequences

- You get speaker labels without a model, but everyone on the far side shares one "Them" label.
- The app requires macOS 26 overall (see the needle ADR), which covers process taps.
- The app needs both microphone and audio capture permissions. A denied microphone permission is detected before recording starts. A denied system audio permission can't be detected: macOS delivers silence instead of an error. The transcript panel shows where to allow it until the other side is heard.
- Each stream starts independently. If one fails, recording continues with the other and the UI names the missing source.
