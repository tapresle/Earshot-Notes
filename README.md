# Earshot Notes

**Meeting notes that never leave your Mac.**

Earshot Notes transcribes your calls, gives you a notepad while you talk, and turns your notes and the transcript into meeting notes. All of it runs on your Mac.

**[Download the latest release](https://github.com/tapresle/Earshot-Notes/releases/latest)** for Macs with Apple Silicon and macOS 26 or later.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/meeting-dark.png">
  <img alt="Earshot Notes showing a meeting's notes beside its live transcript" src="docs/screenshots/meeting-light.png">
</picture>

## Private by design

Your meetings stay on your Mac.

- **On-device:** transcription and summaries run on your Mac. Audio, transcripts, notes, and summaries are stored on it and nowhere else.
- **Nothing to sign up for:** no account, no cloud service, no subscription, and no analytics. The app's logs stay on your Mac too.
- **Works offline:** once a summary model is downloaded, Earshot Notes makes no network requests. The only one it ever makes is the model download you start.
- **No recordings by default:** audio is transcribed and discarded. It's kept only if you turn on **Keep audio recordings** in **Settings**.

## How it's different

| Most AI note takers | Earshot Notes |
|---|---|
| Upload your audio to their servers | Transcribes and summarizes on your Mac |
| Need an account and a subscription | Needs neither |
| Use whichever model the vendor picked | Lets you choose the model, or bring your own |
| Stop working without a connection | Works offline once a model is downloaded |

## What it does

- **Both sides of the call:** a live transcript of your microphone and the other participants, labeled **Me** and **Meeting Participants**.
- **A notepad during the meeting:** type rough notes as you go. They become the outline of the summary.
- **Summaries on request:** during or after the meeting, notes grouped by topic, with the specifics kept and next steps with their owners.
- **Your vocabulary:** list the names your meetings use, and what transcription mishears them as, so transcripts and summaries spell them right.
- **Find and export:** search titles, notes, and transcripts, and export any meeting to Markdown. Untitled meetings are named from their summaries.
- **Out of the way:** Earshot Notes lives in the menu bar, keeps recording with its window closed, and can open at login.

## Bring your own models

You decide which model writes your notes.

- **Choose from the list:** in **Settings**, download Gemma 4 E2B (recommended), Phi-4 mini, Qwen3 4B Instruct, Gemma 4 E4B, Qwen3 8B, Ministral 3 8B, or Ministral 3 14B. Each download is pinned to a checksum and verified before use.
- **Or import your own:** import any GGUF model file you already have.
- **Built for Apple Silicon:** summaries run through [llama.cpp](https://github.com/ggml-org/llama.cpp) with Metal. Every listed model is sized to leave room for your call on a Mac with 16 GB of memory.
- **Transcription included:** [Cactus Whistle](https://huggingface.co/Cactus-Compute/whistle) comes with the app, so transcription works from the first launch. Support for other transcription models is planned.

## How it works

1. Earshot Notes listens to your microphone and to your Mac's audio output.
2. Whistle transcribes each source on your Mac as you talk.
3. When you ask for a summary, your notes and the transcript go to the local model you chose.
4. The model writes your meeting notes, which stay on your Mac with the transcript.

## Get started

1. Check that your Mac has Apple Silicon and macOS 26 or later.
2. Download the disk image from the [latest release](https://github.com/tapresle/Earshot-Notes/releases/latest), open it, and drag **Earshot Notes** to **Applications**.
3. Open Earshot Notes. A checklist asks for two permissions:
   - **Microphone:** for your side of the conversation.
   - **Screen & System Audio Recording:** for the other side of your calls. Earshot Notes records only audio, never your screen. If you decline, recording continues with only your microphone.
4. Download a summary model from the checklist or from **Settings**. Gemma 4 E2B, the recommended model, is 3.1 GB.

What's new in each version is on the [releases page](https://github.com/tapresle/Earshot-Notes/releases).

## Build from source

With Xcode, Rust, Node.js, and CMake installed, run `./scripts/fetch-native.sh`, `npm install`, and `./scripts/build-macos.sh`. [Building Earshot Notes](docs/building.md) covers each step, signing, and notarizing.

## License

Earshot Notes is free software, licensed under the [GNU General Public License, version 3](LICENSE) only (`GPL-3.0-only`).

It bundles the needle engine and the Whistle model (Apache 2.0) and links llama.cpp (MIT). Their licenses, and the notices for every Rust crate and JavaScript package compiled into the app, are in the app bundle under `Contents/Resources/licenses`.
