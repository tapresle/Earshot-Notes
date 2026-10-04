# Capture microphone and system audio as separate streams

- **Status:** Accepted
- **Date:** 2026-10-03

## Context

Commercial meeting note apps transcribe both sides of a call with no bot in the meeting. Whistle has no speaker diarization, and it handles at most 30 seconds of 16 kHz mono audio per pass.

## Decision

Capture the microphone and the system audio output as two independent streams. Label microphone segments "Me" and system audio segments "Them". Each stream is resampled to 16 kHz mono and split on silence into segments of 28 seconds or less.

- **macOS:** Core Audio process taps (macOS 14.4 or later), reached through a small Swift static library with a C ABI.
- **Windows (later):** WASAPI loopback through `cpal`.
- **Linux (later):** a PulseAudio or PipeWire monitor source.

## Alternatives considered

- **Microphone only:** This is simpler. It misses the other side whenever you wear headphones, and it can't tell speakers apart.
- **A diarization model:** This adds a second model and its latency, and it still only produces anonymous speaker labels.

## Consequences

- You get speaker labels without a model, but everyone on the far side shares one "Them" label.
- The app requires macOS 14.4 for system audio. ScreenCaptureKit remains a fallback option for macOS 13.
- The app needs both microphone and audio capture permissions, and it handles a denied permission for each one.
