import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage, type TranscriptionModels, type TranscriptionOption } from "../../lib/api";
import { ModelOrigin } from "../summary/ModelOrigin";

function megabytes(bytes: number): string {
  return `${Math.round(bytes / 1e6)} MB`;
}

function capitalized(message: string): string {
  return `${message.charAt(0).toUpperCase()}${message.slice(1)}`;
}

/** Chooses between Whistle, bundled, and Whisper, downloaded on request (#25). */
export function TranscriptionSection({ recording }: { recording: boolean }) {
  const [state, setState] = useState<TranscriptionModels | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    api
      .transcriptionModels()
      .then(setState)
      .catch((e) => setError(errorMessage(e)));
  }, []);

  useEffect(load, [load]);

  useEffect(() => {
    const update = (id: string, change: (model: TranscriptionOption) => TranscriptionOption) =>
      setState((current) => current && { ...current, models: current.models.map((m) => (m.id === id ? change(m) : m)) });
    const subscriptions = [
      api.onTranscriptionDownloadProgress(({ modelId, ...download }) => update(modelId, (m) => ({ ...m, download }))),
      api.onTranscriptionDownloadDone(load),
      api.onTranscriptionDownloadFailed(({ message, cancelled }) => {
        if (!cancelled) setError(`${capitalized(message)}.`);
        load(); // Picks up the size of the partial file for resuming.
      }),
    ];
    return () => {
      for (const subscription of subscriptions) subscription.then((stop) => stop()).catch(() => {});
    };
  }, [load]);

  async function run(action: () => Promise<void>) {
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(errorMessage(e));
    }
    load();
  }

  async function download(model: TranscriptionOption) {
    setError(null);
    try {
      await api.downloadTranscriptionModel(model.id);
      setState(
        (current) =>
          current && {
            ...current,
            models: current.models.map((m) =>
              m.id === model.id
                ? { ...m, download: { phase: "downloading", received: m.partialBytes, total: m.sizeBytes } }
                : m,
            ),
          },
      );
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <section className="settings-section" aria-labelledby="transcription-heading">
      <h2 id="transcription-heading">Transcription</h2>
      {state?.unavailable && (
        <p className="field-error" role="alert">
          Transcription is unavailable: {state.unavailable}.
        </p>
      )}
      {state && (
        <fieldset className="model-choices">
          <legend className="settings-help">
            Transcripts run on this Mac with the selected model.{" "}
            {recording
              ? "A change applies from the next recording; this one keeps its model."
              : "A change applies from the next recording."}
          </legend>
          {state.models.map((model) => (
            <TranscriptionChoice
              key={model.id}
              model={model}
              selected={state.selectedId === model.id}
              onSelect={() => void run(() => api.selectTranscriptionModel(model.id))}
              onDownload={() => void download(model)}
              onCancel={() => void api.cancelDownload().catch((e) => setError(errorMessage(e)))}
              onRemove={() => run(() => api.removeTranscriptionModel(model.id))}
              onError={setError}
            />
          ))}
        </fieldset>
      )}
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}

function TranscriptionChoice({
  model,
  selected,
  onSelect,
  onDownload,
  onCancel,
  onRemove,
  onError,
}: {
  model: TranscriptionOption;
  selected: boolean;
  onSelect: () => void;
  onDownload: () => void;
  onCancel: () => void;
  onRemove: () => Promise<void>;
  onError: (message: string) => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const [removing, setRemoving] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const removeRef = useRef<HTMLButtonElement>(null);
  const opened = useRef(false);
  const { download } = model;

  // Focus Cancel when the confirmation opens, and return to Remove when it closes.
  useEffect(() => {
    if (confirming) {
      opened.current = true;
      cancelRef.current?.focus();
    } else if (opened.current) {
      removeRef.current?.focus();
    }
  }, [confirming]);

  async function remove() {
    setRemoving(true);
    await onRemove();
    setRemoving(false);
    setConfirming(false);
  }

  const details = model.bundled
    ? "Included with the app"
    : model.installed
      ? `${megabytes(model.sizeBytes)} · Downloaded`
      : `${megabytes(model.sizeBytes)} download`;

  return (
    <div className={selected ? "installed-model selected" : "installed-model"}>
      <label className="model-choice">
        <input
          type="radio"
          name="transcription-model"
          className="sr-only"
          checked={selected}
          disabled={!model.installed}
          onChange={onSelect}
        />
        <span>
          <span className="model-choice-name">
            <strong>{model.displayName}</strong>
            {/* The radio already announces the selection; the badge is for sighted users. */}
            {selected && (
              <span className="badge" aria-hidden="true">
                Selected
              </span>
            )}
          </span>
          <span className="model-details">{details}</span>
          <span className="model-details">{model.description}</span>
        </span>
      </label>
      {!model.bundled &&
        (download?.phase === "downloading" ? (
          <div className="download-progress">
            <progress value={download.received} max={download.total} aria-label={`Downloading ${model.displayName}`} />
            <span role="status">
              {megabytes(download.received)} of {megabytes(download.total)} (
              {Math.floor((download.received / download.total) * 100)}%)
            </span>
            <button type="button" className="button subtle" onClick={onCancel}>
              Cancel
            </button>
          </div>
        ) : download?.phase === "verifying" ? (
          <p role="status" className="download-status">
            Verifying the download…
          </p>
        ) : !model.installed ? (
          <button type="button" className="button primary" onClick={onDownload}>
            {model.partialBytes > 0
              ? `Resume download (${megabytes(model.partialBytes)} of ${megabytes(model.sizeBytes)})`
              : `Download (${megabytes(model.sizeBytes)})`}
          </button>
        ) : confirming ? (
          <div className="model-remove-confirm" role="group" aria-label={`Remove ${model.displayName}`}>
            <span>
              Delete it and free {megabytes(model.sizeBytes)}? Transcripts go back to Whistle, and you can download it
              again later.
            </span>
            <button type="button" className="button danger" onClick={() => void remove()} disabled={removing}>
              {removing ? "Removing…" : "Remove"}
            </button>
            <button
              type="button"
              className="button subtle"
              ref={cancelRef}
              onClick={() => setConfirming(false)}
              disabled={removing}
            >
              Cancel
            </button>
          </div>
        ) : (
          <button
            type="button"
            className="link-button"
            ref={removeRef}
            onClick={() => setConfirming(true)}
            aria-label={`Remove ${model.displayName}`}
          >
            Remove
          </button>
        ))}
      {model.huggingFace && (
        <ModelOrigin modelId={model.id} displayName={model.displayName} source={model.huggingFace} onError={onError} />
      )}
    </div>
  );
}
