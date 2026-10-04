import { useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  errorMessage,
  isCommandError,
  type InputDevice,
  type InstalledModels,
  type ModelRecord,
  type TranscriptionModel,
} from "../../lib/api";
import { ModelSetup } from "../summary/ModelSetup";

export function SettingsView({ onShowSetup }: { onShowSetup: () => void }) {
  const [error, setError] = useState<string | null>(null);

  async function showSetup() {
    try {
      await api.setSetupComplete(false);
      onShowSetup();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <article className="settings">
      <h1>Settings</h1>
      <MicrophoneSection />
      <SummaryModelSection />
      <TranscriptionSection />
      <section className="settings-section">
        <button type="button" className="link-button" onClick={() => void showSetup()}>
          Show the setup checklist again
        </button>
        {error && (
          <p className="field-error" role="alert">
            {error}
          </p>
        )}
      </section>
    </article>
  );
}

const DEFAULT = "";

function MicrophoneSection() {
  const [devices, setDevices] = useState<InputDevice[] | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const load = useCallback(() => {
    Promise.all([api.inputDevices(), api.inputDevice()])
      .then(([list, current]) => {
        setDevices(list);
        setChosen(current ?? null);
        setError(null);
      })
      .catch((e) => setError(errorMessage(e)));
  }, []);

  useEffect(load, [load]);

  async function choose(value: string) {
    const deviceId = value === DEFAULT ? null : value;
    setSaved(false);
    try {
      await api.setInputDevice(deviceId);
      setChosen(deviceId);
      setSaved(true);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  const systemDefault = devices?.find((d) => d.isDefault);
  const missing = chosen !== null && devices !== null && !devices.some((d) => d.id === chosen);

  return (
    <section className="settings-section" aria-labelledby="microphone-heading">
      <h2 id="microphone-heading">Microphone</h2>
      <p className="settings-help">Used for your side of the conversation. Changes apply to the next recording.</p>
      {devices === null && !error && <p className="empty-hint">Looking for microphones…</p>}
      {devices !== null && (
        <div className="settings-row">
          <label htmlFor="microphone">Record from</label>
          <select id="microphone" value={chosen ?? DEFAULT} onChange={(e) => void choose(e.target.value)}>
            <option value={DEFAULT}>
              System default{systemDefault ? ` (${systemDefault.name})` : ""}
            </option>
            {devices.map((device) => (
              <option key={device.id} value={device.id}>
                {device.name}
              </option>
            ))}
            {missing && <option value={chosen}>Previously chosen microphone (not connected)</option>}
          </select>
          <button type="button" className="button subtle" onClick={load}>
            Refresh
          </button>
        </div>
      )}
      {missing && (
        <p className="field-warning">That microphone isn't connected. Recordings use the system default until it is.</p>
      )}
      {saved && (
        <p className="settings-saved" role="status">
          Saved
        </p>
      )}
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}

function gigabytes(bytes: number): string {
  return `${(bytes / 1e9).toFixed(1)} GB`;
}

function SummaryModelSection() {
  const [installed, setInstalled] = useState<InstalledModels | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    api
      .installedModels()
      .then(setInstalled)
      .catch((e) => setError(errorMessage(e)));
  }, []);

  useEffect(load, [load]);

  async function select(model: ModelRecord) {
    setError(null);
    try {
      await api.selectSummaryModel(model.id);
      setInstalled((current) => current && { ...current, selectedId: model.id });
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <section className="settings-section" aria-labelledby="summary-model-heading">
      <h2 id="summary-model-heading">Summary model</h2>
      {installed && installed.models.length > 0 && (
        <fieldset className="model-choices">
          <legend className="settings-help">Summaries use the selected model.</legend>
          {installed.models.map((model) => (
            <InstalledModel
              key={model.id}
              model={model}
              selected={installed.selectedId === model.id}
              onSelect={() => void select(model)}
              onRemoved={load}
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
      {/* Remounts when the installed models change, so download cards show what's installed now. */}
      <ModelSetup key={installed?.models.map((m) => m.id).join() ?? ""} onReady={load} />
    </section>
  );
}

function InstalledModel({
  model,
  selected,
  onSelect,
  onRemoved,
  onError,
}: {
  model: ModelRecord;
  selected: boolean;
  onSelect: () => void;
  onRemoved: () => void;
  onError: (message: string | null) => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const [removing, setRemoving] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const removeRef = useRef<HTMLButtonElement>(null);
  const opened = useRef(false);

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
    onError(null);
    setRemoving(true);
    try {
      await api.removeSummaryModel(model.id);
      onRemoved();
    } catch (e) {
      if (isCommandError(e) && e.kind === "notFound") {
        onRemoved(); // Already gone, so the list is just out of date.
        return;
      }
      onError(errorMessage(e));
      setRemoving(false);
      setConfirming(false);
    }
  }

  return (
    <div className={selected ? "installed-model selected" : "installed-model"}>
      <label className="model-choice">
        <input type="radio" name="summary-model" className="sr-only" checked={selected} onChange={onSelect} />
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
          <span className="model-details">
            {gigabytes(model.sizeBytes)} · {model.source === "download" ? "Downloaded" : "Imported"}
            {model.version ? ` · ${model.version}` : ""}
          </span>
        </span>
      </label>
      {confirming ? (
        <div className="model-remove-confirm" role="group" aria-label={`Remove ${model.displayName}`}>
          <span>
            Delete it and free {gigabytes(model.sizeBytes)}? You can {model.source === "download" ? "download" : "import"}{" "}
            it again later.
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
      )}
    </div>
  );
}

function TranscriptionSection() {
  const [model, setModel] = useState<TranscriptionModel | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .transcriptionModel()
      .then(setModel)
      .catch((e) => setError(errorMessage(e)));
  }, []);

  return (
    <section className="settings-section" aria-labelledby="transcription-heading">
      <h2 id="transcription-heading">Transcription</h2>
      {model && (
        <>
          <p>
            {model.engine}, {model.bundled ? "bundled with the app" : "from your models folder"}. Runs on this Mac.
          </p>
          <p className="settings-help selectable">{model.path}</p>
        </>
      )}
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
