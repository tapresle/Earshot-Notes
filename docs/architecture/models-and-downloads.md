# Models and downloads

[Architecture overview](README.md)

Earshot uses two kinds of model files. Summary and chat models are GGUF files run by llama.cpp. Transcription uses Whistle, bundled with the app, or Whisper large-v3-turbo, an optional download. Every downloadable file is pinned in the app's code to a Hugging Face commit, size, and SHA-256.

## The pinned registry

```mermaid
flowchart TB
    subgraph registry["models::mod.rs (compiled into the app)"]
        known["KNOWN_MODELS: KnownModel entries<br/>id, file name, quantization,<br/>repo, revision, sha256, size,<br/>min_runtime, description, context_limit"]
        order["display_order: the recommended model first,<br/>then by size"]
        turbo["WHISPER_TURBO<br/>pinned the same way,<br/>kept out of KNOWN_MODELS"]
        limit["context_limit(sha256)<br/>registry limit, or 32K for imports"]
    end
    url["KnownModel::url()<br/>huggingface.co/{repo}/resolve/{revision}/{file}"]
    known --> url
    turbo --> url
    known --> order
    known --> limit
```

The UI only ever sends a model ID. The Rust side looks the ID up in the registry and builds the URL, so the window can't make the app download from anywhere else.

## Downloading

```mermaid
sequenceDiagram
    autonumber
    participant UI as Window
    participant Cmd as download_model or<br/>download_transcription_model
    participant DL as models::manager::Downloads
    participant T as model-download thread
    participant HF as Hugging Face
    participant Disk as models/
    participant Store

    UI->>Cmd: model ID
    Cmd->>Cmd: look up the pinned entry (unknown ID: refused)
    Cmd->>DL: start(job, event sink)
    DL-->>Cmd: Busy if any download is running
    DL->>T: spawn
    T->>Disk: size of the .part file, if any
    T->>HF: GET the pinned URL, with Range from that offset
    loop Body read on its own thread
        HF-->>T: bytes
        T->>Disk: append to .part
        T-->>UI: download-progress (every 250 ms)
        Note over T: Checks cancel every 200 ms,<br/>fails after 60 s with no data
    end
    T-->>UI: download-progress (Verifying)
    T->>Disk: check size and SHA-256
    alt matches
        T->>Disk: rename .part to the final name
        T->>Store: add the model row (summary models only)
        T-->>UI: download-done
    else doesn't match
        T->>Disk: delete .part
        T-->>UI: download-failed (corrupt)
    end
```

- The events are named `model:download-*` for summary models and `transcription:download-*` for Whisper. `Downloads` runs one download at a time across both kinds.
- Cancelling or losing the connection keeps the `.part` file, and the next attempt resumes from it.
- A summary model whose SHA-256 is already installed isn't downloaded again.
- Only a verified file is renamed into place. That's why a Whisper file in `models/` counts as installed with no database row.

## Importing a GGUF file

```mermaid
flowchart TB
    pick["You choose a file<br/>(the dialog plugin, in the window)"]
    cmd["import_model(path)"]
    header{"Starts with the GGUF magic?"}
    hash["SHA-256 of the file"]
    already{"Already installed?"}
    knownq{"Matches a registry entry?"}
    asKnown["Use the entry's id, name, version,<br/>file name, and context limit"]
    asLocal["id local-&lt;hash prefix&gt;,<br/>name from the file name,<br/>32K context"]
    copy["Copy into models/<br/>(unique name)"]
    row["Add the models row;<br/>selected only if it's the first model"]
    pick --> cmd --> header
    header -- "no" --> notModel["ImportError::NotAModel"]
    header -- "yes" --> hash --> already
    already -- "yes" --> existing["Return the existing row"]
    already -- "no" --> knownq
    knownq -- "yes" --> asKnown --> copy
    knownq -- "no" --> asLocal --> copy
    copy --> row
```

## Choosing and removing models

```mermaid
flowchart LR
    subgraph summaryModel["Summary and chat model"]
        sel["select_model"] --> unload1["save selected_model_id,<br/>then queue ModelWorker::unload<br/>(after any running job)"]
        rm["remove_model"] --> idle["ModelWorker::when_idle:<br/>unload, delete the file<br/>(only inside models/),<br/>forget the row"]
    end
    subgraph transcriptionModel["Transcription model"]
        selT["select_transcription_model"] --> saveT["save transcription_model<br/>(Whisper only if installed)"] --> pre["TranscriptionWorker::preload:<br/>load Whisper, or free it<br/>when Whistle is chosen"]
        rmT["remove_transcription_model"] --> preW["preload(Whistle) queues<br/>freeing Whisper"] --> delT["remove_whisper: delete the file,<br/>fall back to Whistle"]
    end
```

The next recording reads `transcription_model` when it starts. A recording already running keeps the engine it started with.

## Where the code is

| Concern | Code |
|---|---|
| Registry, display order, import, removal | `src-tauri/src/models/mod.rs` |
| Download, resume, verify | `src-tauri/src/models/download.rs` |
| One download at a time, events | `src-tauri/src/models/manager.rs`, `src-tauri/src/commands/models.rs` |
| Whistle and Whisper selection | `src-tauri/src/models/transcription.rs`, `src-tauri/src/commands/transcription.rs` |
| Download panel and model lists | `src/features/models/`, `src/features/settings/` |
| Checking for newer pins upstream | `scripts/upstream/check.ts` |
