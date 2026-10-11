# Local AI model

[Architecture overview](README.md)

The local AI model writes summaries and answers questions about a meeting. One thread, `model-worker`, owns the model and runs one job at a time, either a summary or a chat question. Both run llama.cpp on the GPU through `llama-cpp-2`.

## Components

```mermaid
flowchart TB
    subgraph ui["Window"]
        summaryTab["Summary tab<br/>useSummary"]
        chatTab["Chat tab<br/>useChat"]
    end

    subgraph cmds["Commands (blocking pool)"]
        summarizeCmd["summarize_meeting<br/>is_final = not recording this meeting"]
        askCmd["ask_meeting"]
        cancelCmds["cancel_summary, cancel_chat,<br/>active_summary, active_chat"]
        settingsCmds["select_model, remove_model,<br/>set_keep_model_loaded"]
    end

    subgraph worker["llm::worker::ModelWorker"]
        api["summarize / ask<br/>refuse if busy (WorkerError::Busy),<br/>read the selected model and a<br/>snapshot of the meeting, then queue"]
        active["Active slot<br/>meeting, Work::Summary or Work::Chat,<br/>cancel flag"]
        thread["model-worker thread<br/>run_worker loop"]
        resident["llm::resident::Resident<br/>one loaded model, keyed by id + SHA-256"]
        api --> active
        api -- "Message::Summary / Message::Chat" --> thread
        thread --> resident
    end

    subgraph jobs["Jobs"]
        sjob["summarize::job::run<br/>summarize, save, name an untitled meeting"]
        cjob["chat::job::run<br/>answer, save the exchange"]
    end

    llama["llm::llama::LlamaGenerator<br/>implements llm::generator::Generator"]
    template["llm::template<br/>the model's own Jinja chat template,<br/>rendered with minijinja,<br/>enable_thinking = false"]
    store[("Store<br/>summaries, chat_messages,<br/>meetings.title")]

    summaryTab --> summarizeCmd --> api
    chatTab --> askCmd --> api
    cancelCmds --> active
    settingsCmds -- "unload, keep_loaded,<br/>when_idle" --> worker
    thread --> sjob
    thread --> cjob
    sjob --> llama
    cjob --> llama
    llama --> template
    sjob --> store
    cjob --> store
    thread -- "summary:progress, summary:text,<br/>summary:done, summary:failed<br/>chat:progress, chat:text,<br/>chat:done, chat:failed" --> ui
```

- **One job at a time.** A summary requested while a question runs, or the reverse, is refused with a message naming what's running.
- **Snapshots, not live reads.** `summarize` and `ask` read the meeting's notes, segments, vocabulary terms, and chat history on the caller's thread and pass them in the job. A job sees the meeting as it was when you asked.
- **The `Generator` trait** separates the summary and chat logic from llama.cpp, so their tests use fakes.

## Model lifecycle

```mermaid
stateDiagram-v2
    [*] --> Unloaded
    Unloaded --> Loading: a job arrives (stage Loading)
    Loading --> Loaded: LlamaGenerator loads the file
    Loading --> Unloaded: load fails (job fails)
    Loaded --> Loaded: job for the same model
    Loaded --> Loading: job for a different model (old one freed first)
    Loaded --> Unloaded: idle for keep_model_loaded<br/>(Never = at once, 5 min default, 15 min)
    Loaded --> Unloaded: select_model or remove_model
    Loaded --> Unloaded: shutdown (quitting)
    Unloaded --> [*]: shutdown
```

- **Idle unloading.** The thread waits on its queue with `recv_timeout` set to the idle limit while a model is loaded. A timeout frees the model. **Always** waits without a limit.
- **Removing a model** runs inside `when_idle`, which waits for the active slot to clear, so a running job never loses its model.
- **Quitting** sets the running job's cancel flag and sends `Shutdown`, then waits up to 3 seconds for the model to be freed. ggml's Metal backend aborts the process at exit while a model is loaded.

## Context and memory

`LlamaGenerator` caps the model's context at the smallest of its training length, `llm::llama::MAX_CONTEXT` (32,768 tokens), and its registry `context_limit` (16K for the 8B models, 8K for Ministral 3 14B; imports get 32K). Each `generate` call creates a fresh llama.cpp context sized to that prompt plus its output room, so short meetings use less memory than long ones.

| Output room | Size |
|---|---|
| Finished notes (`summarize::run::summary_tokens`) | a quarter of the context, at most 4,096 tokens |
| Each condensed part (`part_tokens`) | an eighth of the context, at most 1,024 tokens |
| A chat answer (`chat::answer::answer_tokens`) | a quarter of the context, at most 1,024 tokens |
| Earlier questions and answers sent with a question | an eighth of the context |

## Summaries

```mermaid
flowchart TB
    start["summarize::job::run"]
    load["Resident::get<br/>(Stage::Loading if a load is needed)"]
    single["summary_messages:<br/>title, notes as the outline,<br/>vocabulary terms, every transcript line"]
    fits{"Prompt + notes room<br/>fit the context?"}
    write["generate the notes<br/>(Stage::Writing, streams summary:text)"]
    windows["prompt::windows: split the lines<br/>into parts that fit"]
    parts["part_messages per part<br/>(Stage::Reading part i of n):<br/>specifics first, routine updates last,<br/>the notes mark what matters"]
    combined["combine_messages:<br/>title, notes, the condensed parts"]
    fits2{"Combined prompt fits?"}
    again["Condense the parts again"]
    tooSmall["GenerateError::ContextTooSmall<br/>when condensing can't shrink further"]
    me["note_taker: rewrite 'Me' as 'you'<br/>in streamed and final text"]
    save["Store::insert_summary<br/>(upto_ms, is_final, model id)"]
    title["name_untitled: if the title is still<br/>'Untitled meeting', suggest one<br/>from the summary and save it<br/>only if still untitled"]
    done["summary:done (summary, title)"]

    start --> load --> single --> fits
    fits -- "yes" --> write
    fits -- "no" --> windows --> parts --> combined --> fits2
    fits2 -- "yes" --> write
    fits2 -- "no" --> again --> windows
    windows -. "no progress" .-> tooSmall
    write --> me --> save --> title --> done
```

## Chat

```mermaid
flowchart TB
    start["chat::job::run"]
    load["Resident::get, then Stage::Writing"]
    history["recent_turns: the latest questions<br/>and answers that fit an eighth<br/>of the context, without citations"]
    label["Label every source:<br/>transcript lines S1, S2, ...<br/>notes passages N1, N2, ...<br/>(chat::passages: paragraphs,<br/>list items, headings)"]
    fitsAll{"Whole meeting fits?"}
    retrieve["chat::retrieve::select<br/>score segments and passages against<br/>the question in memory (stop words,<br/>simple stemming, rarer words weigh more),<br/>keep the best within the budget,<br/>add the segment on either side"]
    empty{"Anything selected?"}
    notCovered["NOT_COVERED, without<br/>calling the model"]
    prompt["chat::prompt::messages<br/>sources are data, cite labels<br/>in brackets, such as [S2, N1]"]
    generate["generate (streams chat:text),<br/>'Me' rewritten as 'you'"]
    cite["cite: keep only labels that were sent;<br/>map each to a store::Citation<br/>(segment id and time, or notes passage)"]
    save["Store::insert_chat_exchange<br/>question and answer together"]
    done["chat:done"]

    start --> load --> history --> label --> fitsAll
    fitsAll -- "yes" --> prompt
    fitsAll -- "no" --> retrieve --> empty
    empty -- "no" --> notCovered --> save
    empty -- "yes" --> prompt
    prompt --> generate --> cite --> save --> done
```

- Answers come from the transcript and **My notes**, never from the summary.
- A cancelled or failed question isn't saved. The window puts it back in the question box.
- The window renders each citation as a button: a transcript time scrolls the transcript to the segment (`showSegment`), and **My notes** opens the notes with the passage selected.

## Where the code is

| Concern | Code |
|---|---|
| Worker, queue, busy rules, idle unloading | `src-tauri/src/llm/worker.rs` |
| Keeping one model loaded, `KeepLoaded` | `src-tauri/src/llm/resident.rs` |
| llama.cpp loading, context sizing, sampling, streaming | `src-tauri/src/llm/llama.rs` |
| Chat templates | `src-tauri/src/llm/template.rs` |
| Summary prompts, condensing, titles, "you" for "Me" | `src-tauri/src/summarize/` |
| Chat retrieval, prompt, citations, saving | `src-tauri/src/chat/`, `src-tauri/src/store/chat.rs` |
| Measuring summary quality | `src-tauri/examples/summary_eval.rs`, `scripts/benchmark/run.ts` |
