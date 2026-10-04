# Link the prebuilt needle engine statically

- **Status:** Accepted
- **Date:** 2026-10-03

## Context

The plan assumed you would build needle from source and generate bindings with `bindgen`. The needle GitHub repository only holds the Python package. The engine is published as prebuilt binaries on Hugging Face (`Cactus-Compute/needle3`, Apache-2.0): static libraries, shared libraries, and `needle.h` for each platform, including `windows-x86_64` and `linux-x86_64`.

## Decision

- Link `macos-arm64/libneedle.a` statically through a `needle-sys` crate with hand-written `extern "C"` declarations. The header has about 10 functions, so `bindgen` isn't worth its build cost.
- `scripts/fetch-native.sh` downloads the library and `whistle.cact` at pinned Hugging Face revisions and verifies their SHA-256 checksums. Neither binary is committed. The header and license are committed under `src-tauri/vendor/needle/`.

## Findings from the spike

- The library is arm64 only, depends only on libc and libc++, and contains no network symbols or URLs.
- With engine 3.1.0 and Whistle 2.0.0 on an M-series Mac: load took 4 ms, a 4.3 s clip took about 21 ms, and a 30 s clip took about 99 ms.
- A clip over 30 s returns an error. Silence returns an empty text and language.
- When the output buffer is too small, `needle_transcribe` truncates silently and still reports success. The wrapper detects a full buffer and retries with a larger one.

## Consequences

- You can't patch or audit the engine source. Static linking with pinned checksums keeps the binary reproducible.
- Upgrading means changing the pins in `scripts/fetch-native.sh`. The planned update feature builds on this.
