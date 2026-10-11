# Storage

[Architecture overview](README.md)

Everything Earshot keeps lives in the app data folder (`~/Library/Application Support/com.tedpresley.earshot/`): one SQLite database, the downloaded and imported models, and kept audio when that setting is on. Logs go to `~/Library/Logs/com.tedpresley.earshot/`.

## Files on disk

```mermaid
flowchart LR
    subgraph data["App data folder"]
        db[("earshot.db<br/>SQLite, WAL journal,<br/>foreign keys on")]
        subgraph models["models/"]
            gguf["*.gguf<br/>summary and chat models<br/>(downloaded or imported)"]
            whisperBin["ggml-large-v3-turbo-q5_0.bin<br/>Whisper, when downloaded"]
            part["*.part<br/>an unfinished download,<br/>resumed next time"]
            whistleOverride["whistle.cact<br/>only if a future update<br/>places one here"]
        end
        subgraph rec["recordings/&lt;meeting id&gt;/"]
            wav["&lt;offset&gt;-&lt;source&gt;.wav<br/>16 kHz mono, per source<br/>(Keep audio recordings)"]
        end
    end
    subgraph bundle["App bundle"]
        whistle["Resources/models/whistle.cact"]
        licenses["Resources/licenses/"]
        dylib["Frameworks/libearshot_whisper.dylib"]
    end
    logs["~/Library/Logs/.../Earshot Notes.log<br/>at most 1 MB, one file"]
```

## Database schema

Migrations are numbered SQL files in `src-tauri/migrations/`, applied in order by `store::migrations` using `PRAGMA user_version`.

```mermaid
erDiagram
    meetings ||--o| notes : "has"
    meetings ||--o{ transcript_segments : "has"
    meetings ||--o{ summaries : "has"
    meetings ||--o{ chat_messages : "has"
    models ||..o{ summaries : "model_id (kept after removal)"
    models ||..o{ chat_messages : "model_id"

    meetings {
        INTEGER id PK
        TEXT title
        INTEGER created_at "Unix ms"
        INTEGER started_at "first recording start"
        INTEGER ended_at "recording stop"
    }
    notes {
        INTEGER meeting_id PK,FK
        TEXT body_markdown
        INTEGER updated_at
    }
    transcript_segments {
        INTEGER id PK
        INTEGER meeting_id FK
        TEXT source "me or them"
        INTEGER start_ms "offset from started_at"
        INTEGER end_ms
        TEXT text "vocabulary-corrected"
        TEXT language
        TEXT heard_text "original, when corrected"
    }
    summaries {
        INTEGER id PK
        INTEGER meeting_id FK
        INTEGER upto_ms "transcript covered"
        INTEGER is_final "0 while recording"
        TEXT body_markdown
        TEXT model_id
        INTEGER created_at
    }
    chat_messages {
        INTEGER id PK
        INTEGER meeting_id FK
        TEXT role "user or assistant"
        TEXT body_markdown
        TEXT sources_json "store::Citation array"
        TEXT model_id "NULL for questions"
        INTEGER created_at
    }
    models {
        TEXT id PK
        TEXT display_name
        TEXT file_path
        INTEGER size_bytes
        TEXT sha256 UK
        TEXT source "import or download"
        TEXT source_url
        TEXT version
        TEXT min_runtime
        INTEGER added_at
    }
    settings {
        TEXT key PK
        TEXT value
    }
```

- Every table that belongs to a meeting cascades on delete. Deleting a meeting also deletes its kept audio folder, and it's refused while that meeting is recording.
- `models` has no foreign keys to it. Summaries and answers keep the model's ID after the model is removed.
- Whisper has no row in `models`. It counts as installed when its file is in `models/`.
- At startup, `close_interrupted_recordings` sets `ended_at` on any meeting left recording by a crash or forced quit.

## Settings

Key and value rows in `settings`, read and written through `Store::setting` and `Store::set_setting`.

| Key | Holds | Read when |
|---|---|---|
| `selected_model_id` | the summary and chat model | each summary or question |
| `keep_model_loaded` | `KeepLoaded`: never, 5 or 15 minutes, always | worker start, and when changed |
| `transcription_model` | `whistle` or the Whisper ID | app start, recording start, and when changed |
| `input_device_id` | the chosen microphone | recording start |
| `mic_speech_sensitivity`, `system_speech_sensitivity` | `chunker::Sensitivity` per source | recording start |
| `vocabulary` | terms and mishearings, as JSON | recording start, each saved segment, prompts |
| `keep_audio_recordings` | `on` to keep WAV files | recording start |
| `setup_complete` | whether the setup checklist finished | when the window loads |

Opening at login isn't stored here: it's macOS's own login item setting, read each time.

## Search

```mermaid
flowchart LR
    meetings["meetings.title"] -- "triggers" --> mfts["meetings_fts"]
    notes["notes.body_markdown"] -- "triggers" --> nfts["notes_fts"]
    segs["transcript_segments.text"] -- "triggers" --> tfts["transcript_fts"]
    query["list_meetings(query, cursor)"] --> terms["store::search: each word<br/>quoted as a prefix"]
    terms --> mfts
    terms --> nfts
    terms --> tfts
    mfts --> union["meetings matching every word,<br/>in any of the three"]
    nfts --> union
    tfts --> union
    union --> page["one page, newest first,<br/>keyed on (created_at, id)"]
```

- The three FTS5 tables are external-content indexes with the `unicode61 remove_diacritics 2` tokenizer, so search ignores case and accents.
- Triggers keep them in step with inserts, updates, and deletes. A migration that rebuilds one of those tables must recreate its index and triggers.
- Paging uses a cursor on `(created_at, id)` with the `meetings_page` index, not an offset.

## Concurrency

`Store` holds one `rusqlite::Connection` behind a `Mutex`. Every caller, whether a command on the blocking pool, the transcription worker, or the model worker, takes the lock for its query or transaction. Writes that must agree with what they read, such as saving a transcribed segment or reapplying the vocabulary, run in a single transaction under that lock.

## Where the code is

| Concern | Code |
|---|---|
| Connection, migrations | `src-tauri/src/store/mod.rs`, `src-tauri/src/store/migrations.rs` |
| Meetings, notes, paging | `src-tauri/src/store/meetings.rs` |
| Segments, recording start and end | `src-tauri/src/store/transcript.rs`, `src-tauri/src/store/transcribed.rs` |
| Search | `src-tauri/src/store/search.rs` |
| Summaries, chat, models, settings, vocabulary | `src-tauri/src/store/summaries.rs`, `chat.rs`, `models.rs`, `settings.rs`, `vocabulary.rs` |
| Export queries | `src-tauri/src/store/export.rs` |
| Kept audio files | `src-tauri/src/recordings.rs` |
