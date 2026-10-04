# Embed llama.cpp for summaries

- **Status:** Accepted
- **Date:** 2026-10-03

## Context

Summaries must run locally with no network connection. Needle, which ships alongside Whistle, is a tool-calling model and can't write meeting summaries. The summary runtime needs GPU acceleration on macOS and a path to Windows and Linux.

## Decision

Link llama.cpp into the Rust core through the `llama-cpp-2` crate. It uses Metal on macOS, and CUDA or Vulkan on Windows and Linux. Models are GGUF files. You provide the model by importing a local file or through a one-time, explicitly started download.

## Alternatives considered

- **Ollama:** This needs the least code, but you would have to install and run a separate service.
- **Cactus engine:** It comes from the same vendor as Whistle and supports Gemma, Qwen, and LFM. Desktop support for Windows and Linux isn't documented yet.

## Consequences

- The app has no external runtime dependency, and inference stays in-process.
- The LLM (about 2.5 GB for a 4B model at Q4) is too large to bundle, so first-run setup includes getting a model.
- Long meetings can exceed the model's context window. Summarization uses map-reduce over transcript windows when that happens.
