# User interface

[Architecture overview](README.md)

The window is a React 19 app in TypeScript, built by Vite and run in the system webview. It has no router and no state library: `App.tsx` holds the shared state, and each feature under `src/features/` keeps its components, hooks, and tests together. Every call to the Rust core goes through `src/lib/api.ts`.

## Layout and components

```mermaid
flowchart LR
    subgraph shell["App.tsx: three columns, two for Settings and setup"]
        direction LR
        subgraph left["Sidebar"]
            list["MeetingList<br/>grouped by day,<br/>multi-select like Finder"]
            search["SearchField<br/>(debounced)"]
            more["LoadMore<br/>(cursor paging)"]
            exportSel["ExportSelectionBar"]
            settingsBtn["Settings button"]
            menu["ContextMenu<br/>Export and Delete,<br/>only Export for a multi-selection"]
        end
        deleteDialog["DeleteMeetingDialog<br/>modal, from the menu<br/>or the meeting's Delete"]
        subgraph main["Workspace: one view at a time"]
            meeting["MeetingView"]
            settings["SettingsView"]
            setup["SetupChecklist"]
        end
        transcript["TranscriptPanel<br/>live while recording,<br/>shown only beside a meeting"]
    end

    subgraph mv["MeetingView"]
        title["TitleField"]
        controls["RecordingControls<br/>(consent reminder)"]
        exportOne["ExportMeetingButton"]
        tabs["Tabs: My notes, Summary, Chat<br/>(panels stay mounted)"]
        notepad["Notepad<br/>autosaves after 500 ms"]
        summary["SummaryPanel<br/>Markdown, ModelSetup"]
        chat["ChatPanel<br/>Markdown with citation buttons,<br/>ModelSetup"]
        tabs --> notepad
        tabs --> summary
        tabs --> chat
    end
    meeting --> mv

    subgraph sv["SettingsView sections, in order"]
        s1["Startup (open at login, setup)"]
        s2["Microphone, speech detection"]
        s3["Transcription (Whistle or Whisper)"]
        s4["Vocabulary, software terms"]
        s5["Audio recordings (keep, size, delete)"]
        s6["Local AI model, Keep loaded,<br/>ModelSetup"]
        s7["Updates, export all"]
    end
    settings --> sv

    chat -- "showSegment: scroll and<br/>highlight a cited line" --> transcript
    chat -- "NotepadHandle.select:<br/>select a cited passage" --> notepad
```

- **State lives in hooks.** `useMeetingList` pages and searches the list, `useSummary` and `useChat` follow their jobs through events, and `useAutosave` saves notes in order without losing the last edit.
- **Model output is Markdown, never HTML.** `features/summary/Markdown.tsx` renders summaries and answers; its `citation` prop turns `[S3]` and `[N1]` groups into buttons.
- **`ModelSetup`** (the download panel) renders only in the visible tab, so the Summary and Chat tabs never both listen to download events.

## Talking to the core

```mermaid
sequenceDiagram
    participant C as Component or hook
    participant A as src/lib/api.ts
    participant T as Tauri IPC
    participant R as Rust command

    C->>A: api.summarizeMeeting(id)
    A->>T: invoke("summarize_meeting", { meetingId })
    T->>R: commands::summaries::summarize_meeting
    R-->>T: Ok(()) or CommandError { kind, message }
    T-->>A: resolve or reject
    A-->>C: promise
    Note over C,A: Long work reports back through events
    C->>A: api.onSummaryText(handler)
    A->>T: listen("summary:text")
    R--)T: emit("summary:text", { meetingId, text })
    T--)A: event
    A--)C: handler(payload)
```

`api.ts` declares about 60 commands and these events:

| Events | Sent by | Used by |
|---|---|---|
| `transcript:segment`, `transcript:removed` | the transcription callback in `lib.rs` | `TranscriptPanel` |
| `recording:changed` | `commands::recording` | `App` |
| `recording:issue` | `commands::recording`, from the capture threads | `RecordingControls` |
| `summary:progress`, `summary:text`, `summary:done`, `summary:failed` | the model worker | `useSummary`; on `summary:done`, `App` and `MeetingView` also pick up a new title |
| `chat:progress`, `chat:text`, `chat:done`, `chat:failed` | the model worker | `useChat` |
| `model:download-progress`, `model:download-done`, `model:download-failed` | `models::manager` | `ModelSetup` (Summary tab, Chat tab, Settings) |
| `transcription:download-progress`, `transcription:download-done`, `transcription:download-failed` | `models::manager` | `TranscriptionSection` |

File pickers (`pickModelFile`, `pickSavePath`, `pickFolder`) use the Tauri dialog plugin from `api.ts`. The chosen path then goes to a command, such as `import_model` or `export_meeting`.

## Tests, previews, and screenshots

```mermaid
flowchart LR
    fake["src/testing/fakeBackend.ts<br/>in-memory commands and events"]
    tests["App.test.tsx and feature tests<br/>(Vitest, Testing Library)"]
    preview["preview.html + src/preview/main.tsx<br/>mockIPC answered by the fake backend"]
    scenes["src/preview/scenes.ts<br/>synthetic scenes"]
    shots["screenshots/scenes.spec.ts<br/>(Playwright: light and dark)"]
    images["docs/screenshots/*.png<br/>committed on purpose"]

    fake --> tests
    fake --> preview
    scenes --> preview
    preview --> shots --> images
```

The real `App` renders in all three. Only `index.html` is bundled into the app, so the fake backend and preview never ship.

## Where the code is

| Concern | Code |
|---|---|
| Shell, shared state, view switching | `src/App.tsx` |
| Commands, events, types | `src/lib/api.ts` |
| Meeting list, search, selection, meeting view | `src/features/meetings/` |
| Notes, transcript, summary, chat | `src/features/notepad/`, `transcript/`, `summary/`, `chat/` |
| Recording controls and consent | `src/features/recording/` |
| Settings, setup, model download panel, export | `src/features/settings/`, `setup/`, `models/`, `export/` |
| Styling | `src/styles.css` (custom properties, light and dark) |
