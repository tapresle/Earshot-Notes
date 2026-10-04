import { useEffect, useState } from "react";
import { api, errorMessage, type RecommendedModel, type SummaryStage } from "../../lib/api";
import { formatOffset } from "../transcript/formatOffset";
import { Markdown } from "./Markdown";
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

export function SummaryPanel({ state }: { state: SummaryState }) {
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
        <ModelSetup onImported={state.setModel} />
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

function formatSize(bytes: number): string {
  return `${(bytes / 1e9).toFixed(1)} GB`;
}

/** Shown until a summary model is installed. */
function ModelSetup({ onImported }: { onImported: SummaryState["setModel"] }) {
  const [recommended, setRecommended] = useState<RecommendedModel | null>(null);
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .recommendedModel()
      .then(setRecommended)
      .catch((e) => console.error("couldn't load the recommended model", e));
  }, []);

  async function importFile() {
    setError(null);
    try {
      const path = await api.pickModelFile();
      if (!path) return;
      setImporting(true);
      onImported(await api.importModel(path));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setImporting(false);
    }
  }

  return (
    <div className="model-setup">
      <h2>Add a summary model</h2>
      <p>Summaries run on this Mac with a local language model, so nothing leaves your computer.</p>
      {recommended && (
        <p>
          The recommended model is {recommended.displayName} ({formatSize(recommended.sizeBytes)}). Download{" "}
          <code>{recommended.fileName}</code> from <span className="selectable">{recommended.url}</span>, then import
          it here. In-app downloads are coming soon.
        </p>
      )}
      <button type="button" className="button primary" onClick={() => void importFile()} disabled={importing}>
        {importing ? "Importing… this can take a minute" : "Import model file"}
      </button>
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </div>
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
