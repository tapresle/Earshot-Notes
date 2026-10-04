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
- The recommended model is Gemma 4 E2B (`gemma-4-E2B-it-Q4_K_M.gguf`, 3.1 GB, Apache 2.0), pinned by Hugging Face revision and SHA-256 in `src-tauri/src/models/mod.rs`.
- Alternatives that fit a 16 GB Mac are offered alongside it, all Q4_K_M under Apache 2.0 or MIT: Qwen3 4B Instruct 2507, Phi-4 mini, Gemma 4 E4B, Qwen3 8B, Ministral 3 8B, and Ministral 3 14B. Each sets a context limit in the registry so its key-value cache stays within the memory Metal can use: 16K tokens for the 8B models and 8K for the 14B. Longer meetings are condensed in parts.
- Prompts are formatted by rendering each model's own Jinja chat template with `minijinja` and `minijinja-contrib`. llama.cpp's built-in template engine doesn't recognize Gemma 4's format, and a hand-written format would break for imported models.
- The model loads for each summary and unloads when it finishes, so it doesn't hold about 3 GB while you're in a call. Each summary pays a load time of a few seconds.
- Summaries stream into the UI. Model output renders through a small Markdown renderer that builds React elements and never injects HTML, so text in a transcript can't inject markup.
- The LLM (3.1 GB for Gemma 4 E2B at Q4) is too large to bundle, so first-run setup includes getting a model.
- Long meetings can exceed the model's context window. Summarization uses map-reduce over transcript windows when that happens.
