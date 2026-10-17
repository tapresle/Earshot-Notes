# Recording and transcription

[Architecture overview](README.md)

A recording captures two sources at once: **Me** (the microphone) and **Them** (the Mac's system audio output, which carries the call). Each source gets its own capture thread and pipeline thread. Both feed one transcription worker, which saves segments and pushes them to the window.

## Data flow

```mermaid
flowchart LR
    subgraph me["Source: Me"]
        micdev["Microphone<br/>(chosen device or default)"]
        miccap["audio::mic<br/>cpal input stream"]
        micpipe["audio-pipeline-Me"]
    end
    subgraph them["Source: Them"]
        outdev["Default output device"]
        tap["audio::system<br/>Core Audio process tap<br/>(macos-audio crate)"]
        follower["Follower<br/>rebuilds the tap when the<br/>output device changes,<br/>fills the gap with silence"]
        syspipe["audio-pipeline-Them"]
    end

    micdev --> miccap
    outdev --> tap --> follower
    miccap -- "audio::Buffer<br/>(samples + capture rate)<br/>bounded channel, drops when full" --> micpipe
    follower -- "audio::Buffer" --> syspipe

    subgraph pipeline["recording::run_pipeline (per source)"]
        direction TB
        downmix["downmix to mono"]
        resample["audio::resample::To16k<br/>new resampler when the rate changes"]
        chunker["chunker::Chunker<br/>ends a segment after a 2 s pause<br/>(160 ms once past 20 s), at most 28 s;<br/>measures speech time and loudness"]
        downmix --> resample --> chunker
    end
    micpipe -.-> pipeline
    syspipe -.-> pipeline

    kept["recordings::KeptAudio<br/>WAV writer thread<br/>(Keep audio recordings)"]
    resample -- "16 kHz stream" --> kept

    worker["transcribe::worker::TranscriptionWorker<br/>one thread, one queue"]
    chunker -- "Job: meeting, source,<br/>start/end ms, 16 kHz clip,<br/>speech stats, keywords, engine" --> worker

    subgraph engines["Engines"]
        whistle["transcribe::whistle<br/>Whistle on needle<br/>keyword biasing"]
        whisperEng["transcribe::whisper<br/>Whisper large-v3-turbo<br/>strips [BLANK_AUDIO] labels"]
    end
    worker --> whistle
    worker --> whisperEng

    cleanup["transcribe::cleanup<br/>removes fillers such as 'um',<br/>drops results left empty"]
    whistle --> cleanup
    whisperEng --> cleanup
    phantom["transcribe::phantom<br/>drops 'Thank you.' and similar<br/>from weak clips"]
    cleanup --> phantom

    save["Store::insert_transcribed_segment<br/>vocabulary corrections,<br/>echo removal"]
    phantom --> save
    db[("transcript_segments")]
    save --> db
    ui["Window"]
    save -- "transcript:segment<br/>transcript:removed" --> ui
```

## Starting and stopping a recording

Every start and stop goes through `commands::recording::start` and `stop`, never `recording::start_recording` directly. Those commands update the menu bar and emit `recording:changed`, which is how the window learns about a recording started from the menu bar.

```mermaid
sequenceDiagram
    autonumber
    participant UI as Window or menu bar
    participant Cmd as commands::recording
    participant Rec as recording::start_recording
    participant Store
    participant Cap as Capture threads
    participant Pipe as Pipeline threads
    participant TW as TranscriptionWorker

    UI->>Cmd: start_recording(meeting)
    Cmd->>Store: read transcription_model
    Cmd->>Rec: start(sources, engine)
    Rec->>Store: offset, keep audio setting, vocabulary keywords
    loop Me, then Them
        Rec->>Store: speech sensitivity for the source
        Rec->>Cap: start capture
        Cap-->>Rec: format (rate, channels) or an error
    end
    Note over Rec: A source that fails is reported,<br/>recording continues with the other.
    Rec->>Store: begin_recording (sets started_at)
    Rec->>TW: preload(engine)
    Rec->>Pipe: spawn one pipeline per source
    Cmd-->>UI: recording:changed, tray icon shows recording

    loop While recording
        Cap->>Pipe: audio::Buffer
        Pipe->>TW: submit(Job) per finished segment
        TW->>Store: insert_transcribed_segment
        TW-->>UI: transcript:segment / transcript:removed
    end

    UI->>Cmd: stop_recording
    Cmd->>Rec: stop
    Rec->>Cap: drop captures (streams stop)
    Pipe->>TW: flush the last segment
    Rec->>Store: end_recording (sets ended_at)
    Cmd-->>UI: recording:changed
```

## The transcription worker

```mermaid
flowchart TB
    queue["Queue of Jobs<br/>(both sources)"]
    pick{"job.engine"}
    queue --> pick
    pick -- "Whistle" --> w1["Whistle<br/>loaded at app start,<br/>stays loaded"]
    pick -- "Whisper" --> loaded{"Whisper loaded?"}
    loaded -- "yes" --> w2["Whisper<br/>loaded at app start when chosen,<br/>stays loaded (about 1 GB)"]
    loaded -- "load fails" --> w1
    w1 --> result["Transcription<br/>text, language, engine used"]
    w2 --> result
    result --> callback["on_segment callback (lib.rs)<br/>saves and emits"]
```

- needle holds one process-global, non-thread-safe model, so Whistle lives only on this thread.
- Whistle and Whisper both receive 16 kHz mono clips of at most 28 seconds.
- **Settings** > **Transcription** loads Whisper when chosen and frees it when you choose Whistle or remove the file.

## Saving a segment

`Store::insert_transcribed_segment` (`store/transcribed.rs`) is the only way segments are saved. It runs in one transaction on the store's connection, so the vocabulary and the nearby segments it reads can't change mid-save.

```mermaid
flowchart TB
    input["New segment from the worker<br/>(source, times, text heard)"]
    source{"Source"}
    input --> source

    subgraph mepath["Me"]
        theirs["Read Them lines within 2 s<br/>of this segment"]
        isEcho{"transcribe::echo::is_echo<br/>on the vocabulary-corrected text:<br/>3+ words, 70% found in order<br/>in Them's lines?"}
        strip["echo::without_echo<br/>drop runs of 4+ words<br/>repeated from Them"]
        empty{"Anything left?"}
        correctMe["vocabulary: replace mishearings;<br/>keep the original in heard_text"]
        theirs --> isEcho
        isEcho -- "no" --> strip --> correctMe --> empty
    end

    subgraph thempath["Them"]
        correctThem["vocabulary: replace mishearings;<br/>keep the original in heard_text"]
        insertThem["Insert the segment"]
        earlier["remove_echoes_of: earlier Me<br/>segments within 2 s that echo it<br/>are deleted or stripped"]
        correctThem --> insertThem --> earlier
    end

    source -- "Me" --> theirs
    source -- "Them" --> correctThem
    drop["Not saved"]
    isEcho -- "yes" --> drop
    empty -- "no" --> drop
    insertMe["Insert the segment"]
    empty -- "yes" --> insertMe

    events["lib.rs emits transcript:segment<br/>for new and updated segments,<br/>transcript:removed for deleted ones"]
    insertMe --> events
    earlier --> events
```

Without headphones, participants' words come back through the microphone as **Me**. Echo removal compares text, not audio, because the two sources close segments on their own pauses and either can arrive first.

## Where the code is

| Concern | Code |
|---|---|
| Start, stop, and the pipeline loop | `src-tauri/src/recording.rs` |
| Microphone capture and device list | `src-tauri/src/audio/mic.rs` |
| System audio tap and output-device following | `src-tauri/src/audio/system.rs`, `src-tauri/crates/macos-audio` |
| Resampling to 16 kHz | `src-tauri/src/audio/resample.rs` |
| Speech segmentation and sensitivity | `src-tauri/src/chunker.rs` |
| Kept audio | `src-tauri/src/recordings.rs` |
| Worker, engines, filler cleanup, phantom filter, echo | `src-tauri/src/transcribe/` |
| Vocabulary terms and corrections | `src-tauri/src/vocabulary.rs`, `src-tauri/src/store/transcribed.rs` |
