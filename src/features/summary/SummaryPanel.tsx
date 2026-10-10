import { useState } from "react";
import type { SummaryStage } from "../../lib/api";
import { formatOffset } from "../transcript/formatOffset";
import { Markdown } from "./Markdown";
import { ModelSetup } from "../models/ModelSetup";
import type { SummaryState } from "./useSummary";

const stageText = (stage: SummaryStage): string => {
  switch (stage.stage) {
    case "loading":
      return "Loading the model…";
    case "reading":
      return `Reading the transcript, part ${stage.part} of ${stage.parts}…`;
    case "writing":
      return "Writing notes…";
  }
};

/**
 * `active` is whether the Summary tab is showing. The model setup renders only
 * then, because the Chat tab shows the same setup and only one copy may
 * listen for downloads at a time.
 */
export function SummaryPanel({ state, active }: { state: SummaryState; active: boolean }) {
  const { loading, loadError, summary, model, run } = state;

  if (loading) return <p className="empty-hint">Loading summary…</p>;
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

  if (run.kind === "running") {
    return (
      <section className="summary" aria-busy="true">
        <div className="summary-status">
          <span role="status">{stageText(run.stage)}</span>
          <button type="button" className="button subtle" onClick={state.cancel}>
            Cancel
          </button>
        </div>
        {run.text && <Markdown source={run.text} />}
      </section>
    );
  }

  return (
    <section className="summary">
      {run.kind === "failed" && (
        <p className={run.cancelled ? "field-warning" : "field-error"} role="alert">
          {run.cancelled ? "Summary cancelled." : `Couldn't write the summary: ${run.message}.`}
        </p>
      )}
      {!model ? (
        active && <ModelSetup onReady={state.setModel} />
      ) : summary ? (
        <>
          <p className="summary-meta">
            {summary.isFinal
              ? "Summary of the whole meeting"
              : `Partial summary, up to ${formatOffset(summary.uptoMs)} into the meeting`}
          </p>
          <Markdown source={summary.bodyMarkdown} />
        </>
      ) : (
        <p className="empty-hint">
          No summary yet. Click <strong>Summarize</strong> to turn your notes and the transcript into meeting notes.
        </p>
      )}
    </section>
  );
}

export function SummarizeButton({
  state,
  recording,
  onStarted,
}: {
  state: SummaryState;
  recording: boolean;
  onStarted: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const running = state.run.kind === "running";

  async function summarize() {
    setError(null);
    onStarted();
    if (!state.model) return; // The summary tab explains how to add one.
    setError(await state.start());
  }

  return (
    <div className="summarize-control">
      <button type="button" className="button" onClick={() => void summarize()} disabled={running || state.loading}>
        {running ? "Summarizing…" : recording ? "Summarize so far" : "Summarize"}
      </button>
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
