import { type FormEvent, type KeyboardEvent, useEffect, useRef, useState } from "react";
import type { ChatMessage, ChatStage, Citation, ModelRecord } from "../../lib/api";
import { Markdown, type RenderCitation } from "../summary/Markdown";
import { ModelSetup } from "../summary/ModelSetup";
import { formatOffset } from "../transcript/formatOffset";
import { showSegment } from "../transcript/showSegment";
import type { ChatState } from "./useChat";

/** Matches `MAX_QUESTION_CHARS` in `chat/job.rs`. */
const MAX_QUESTION_CHARS = 2000;

const stageText: Record<ChatStage["stage"], string> = {
  loading: "Loading the model…",
  writing: "Writing an answer…",
};

interface ChatPanelProps {
  state: ChatState;
  model: ModelRecord | null;
  onModelReady: (model: ModelRecord) => void;
  /** Whether the Chat tab is showing; the model setup renders only then (see `SummaryPanel`). */
  active: boolean;
  /** Opens **My notes** at a cited passage. */
  onShowNotes: (passage: string) => void;
}

export function ChatPanel({ state, model, onModelReady, active, onShowNotes }: ChatPanelProps) {
  const { loading, loadError, messages, run } = state;
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView?.({ block: "end" });
  }, [messages.length, run]);

  if (loading) return <p className="empty-hint">Loading chat…</p>;
  if (loadError) {
    return (
      <div className="inline-error" role="alert">
        <p>{loadError}</p>
        <button type="button" className="button" onClick={state.reload}>
          Try again
        </button>
      </div>
    );
  }
  if (!model) return active ? <ModelSetup onReady={onModelReady} /> : null;

  // While an answer streams, its sources aren't known yet, so labels are hidden.
  const hideCitations: RenderCitation = () => null;

  return (
    <section className="chat" aria-label="Chat">
      {messages.length === 0 && run.kind !== "running" && (
        <p className="empty-hint">
          Ask a question about this meeting. Answers come from the transcript and your notes, and link to where they
          came from.
        </p>
      )}
      <ol className="chat-messages">
        {messages.map((message) => (
          <li key={message.id} className={`chat-message chat-${message.role}`}>
            <MessageBody message={message} onShowNotes={onShowNotes} />
          </li>
        ))}
        {run.kind === "running" && (
          <>
            {run.question && (
              <li className="chat-message chat-user">
                <p>{run.question}</p>
              </li>
            )}
            <li className="chat-message chat-assistant" aria-busy="true">
              <div className="chat-status">
                <span role="status">{stageText[run.stage.stage]}</span>
                <button type="button" className="button subtle" onClick={state.cancel}>
                  Cancel
                </button>
              </div>
              {run.text && <Markdown source={run.text} citation={hideCitations} />}
            </li>
          </>
        )}
      </ol>
      <div ref={endRef} />
      <Composer state={state} />
    </section>
  );
}

function MessageBody({ message, onShowNotes }: { message: ChatMessage; onShowNotes: (passage: string) => void }) {
  if (message.role === "user") {
    return (
      <p>
        <span className="sr-only">You asked: </span>
        {message.bodyMarkdown}
      </p>
    );
  }
  const citation: RenderCitation = (labels, key) => {
    const cited = labels
      .map((label) => message.sources.find((source) => source.label === label))
      .filter((source): source is Citation => source !== undefined);
    if (cited.length === 0) return null;
    return (
      <span key={key} className="citations">
        {cited.map((source) => (
          <CitationButton key={source.label} source={source} onShowNotes={onShowNotes} />
        ))}
      </span>
    );
  };
  return <Markdown source={message.bodyMarkdown} citation={citation} />;
}

function CitationButton({ source, onShowNotes }: { source: Citation; onShowNotes: (passage: string) => void }) {
  if (source.kind === "transcript") {
    const time = formatOffset(source.startMs);
    return (
      <button
        type="button"
        className="citation"
        aria-label={`Show ${time} in the transcript`}
        onClick={() => showSegment(source.segmentId, source.startMs)}
      >
        {time}
      </button>
    );
  }
  return (
    <button
      type="button"
      className="citation"
      aria-label="Show this in My notes"
      onClick={() => onShowNotes(source.text)}
    >
      My notes
    </button>
  );
}

function Composer({ state }: { state: ChatState }) {
  const { draft, setDraft } = state;
  const [error, setError] = useState<string | null>(null);
  const running = state.run.kind === "running";
  const failed = state.run.kind === "failed" ? state.run : null;

  async function submit(event?: FormEvent) {
    event?.preventDefault();
    const question = draft.trim();
    if (!question || running) return;
    setError(null);
    setDraft("");
    const problem = await state.ask(question);
    if (problem) {
      setError(problem);
      setDraft(question);
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      void submit();
    }
  }

  return (
    <form className="chat-composer" onSubmit={(event) => void submit(event)}>
      {failed && (
        <p className={failed.cancelled ? "field-warning" : "field-error"} role="alert">
          {failed.cancelled ? "Answer cancelled." : `Couldn't answer: ${failed.message}.`}
        </p>
      )}
      <label htmlFor="chat-question" className="sr-only">
        Ask about this meeting
      </label>
      <textarea
        id="chat-question"
        className="chat-input"
        value={draft}
        rows={2}
        maxLength={MAX_QUESTION_CHARS}
        placeholder="Ask about this meeting, such as “What did we decide about the launch date?”"
        aria-invalid={error ? true : undefined}
        aria-describedby={error ? "chat-question-error" : "chat-question-help"}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={onKeyDown}
      />
      <div className="chat-actions">
        <p id="chat-question-help" className="settings-help">
          Press Return to ask, and Shift-Return for a new line.
        </p>
        <button type="submit" className="button" disabled={running || !draft.trim()}>
          Ask
        </button>
      </div>
      {error && (
        <p id="chat-question-error" className="field-error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}
