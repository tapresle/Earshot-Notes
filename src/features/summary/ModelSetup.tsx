import { useCallback, useEffect, useId, useState } from "react";
import { api, errorMessage, type AvailableModel, type ModelRecord } from "../../lib/api";
import { ModelOrigin } from "./ModelOrigin";

function gigabytes(bytes: number): string {
  return `${(bytes / 1e9).toFixed(1)} GB`;
}

/** Shown until a summary model is installed: download a known model or import a file. */
export function ModelSetup({ onReady }: { onReady: (model: ModelRecord) => void }) {
  const [models, setModels] = useState<AvailableModel[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);

  const refresh = useCallback(() => {
    api
      .availableModels()
      .then((list) => {
        setModels(list);
        setLoadError(null);
      })
      .catch((e) => setLoadError(errorMessage(e)));
  }, []);

  useEffect(refresh, [refresh]);

  useEffect(() => {
    const update = (id: string, change: (model: AvailableModel) => AvailableModel) =>
      setModels((list) => list?.map((m) => (m.id === id ? change(m) : m)) ?? list);
    const subscriptions = [
      api.onDownloadProgress(({ modelId, ...download }) => update(modelId, (m) => ({ ...m, download }))),
      api.onDownloadDone(({ model }) => onReady(model)),
      api.onDownloadFailed(({ modelId, message, cancelled }) => {
        if (!cancelled) setError(`${message.charAt(0).toUpperCase()}${message.slice(1)}.`);
        refresh(); // Picks up the size of the partial file for resuming.
        update(modelId, (m) => ({ ...m, download: null }));
      }),
    ];
    return () => {
      for (const subscription of subscriptions) subscription.then((stop) => stop()).catch(() => {});
    };
  }, [onReady, refresh]);

  async function download(model: AvailableModel) {
    setError(null);
    try {
      await api.downloadModel(model.id);
      setModels((list) =>
        list?.map((m) =>
          m.id === model.id
            ? { ...m, download: { phase: "downloading", received: m.partialBytes, total: m.sizeBytes } }
            : m,
        ) ?? list,
      );
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function importFile() {
    setError(null);
    try {
      const path = await api.pickModelFile();
      if (!path) return;
      setImporting(true);
      onReady(await api.importModel(path));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setImporting(false);
    }
  }

  return (
    <div className="model-setup">
      <h2>Add a summary model</h2>
      <p>
        Summaries run on this Mac with a local language model, so nothing leaves your computer. The model downloads
        once from Hugging Face and is checked against its published fingerprint before it's used.
      </p>
      {loadError && (
        <p className="field-error" role="alert">
          {loadError}
        </p>
      )}
      {models?.map((model) => (
        <ModelCard
          key={model.id}
          model={model}
          onDownload={() => void download(model)}
          onCancel={() => void api.cancelDownload().catch((e) => setError(errorMessage(e)))}
          onError={setError}
        />
      ))}
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
      <p className="model-import">
        Already have a GGUF file?{" "}
        <button type="button" className="link-button" onClick={() => void importFile()} disabled={importing}>
          {importing ? "Importing… this can take a minute" : "Import model file"}
        </button>
      </p>
    </div>
  );
}

function ModelCard({
  model,
  onDownload,
  onCancel,
  onError,
}: {
  model: AvailableModel;
  onDownload: () => void;
  onCancel: () => void;
  onError: (message: string) => void;
}) {
  const { download } = model;
  const nameId = useId();
  return (
    <div className="model-card" role="group" aria-labelledby={nameId}>
      <div className="model-card-header">
        <strong id={nameId}>{model.displayName}</strong>
        {model.recommended && <span className="badge">Recommended</span>}
        <span className="model-size">{gigabytes(model.sizeBytes)}</span>
      </div>
      <p className="model-description">{model.description}</p>
      <ModelOrigin modelId={model.id} displayName={model.displayName} source={model.huggingFace} onError={onError} />
      {download?.phase === "downloading" ? (
        <div className="download-progress">
          <progress
            value={download.received}
            max={download.total}
            aria-label={`Downloading ${model.displayName}`}
          />
          <span role="status">
            {gigabytes(download.received)} of {gigabytes(download.total)} (
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
      ) : model.installed ? (
        <p className="download-status">Installed</p>
      ) : (
        <button type="button" className="button primary" onClick={onDownload}>
          {model.partialBytes > 0
            ? `Resume download (${(model.partialBytes / 1e9).toFixed(1)} of ${gigabytes(model.sizeBytes)})`
            : `Download (${gigabytes(model.sizeBytes)})`}
        </button>
      )}
    </div>
  );
}
