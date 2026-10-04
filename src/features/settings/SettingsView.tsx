import { useCallback, useEffect, useState } from "react";
import {
  api,
  errorMessage,
  type InputDevice,
  type InstalledModels,
  type ModelRecord,
  type TranscriptionModel,
} from "../../lib/api";
import { ModelSetup } from "../summary/ModelSetup";

export function SettingsView() {
  return (
    <article className="settings">
      <h1>Settings</h1>
      <MicrophoneSection />
      <SummaryModelSection />
      <TranscriptionSection />
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
            <label key={model.id} className="model-choice">
              <input
                type="radio"
                name="summary-model"
                checked={installed.selectedId === model.id}
                onChange={() => void select(model)}
              />
              <span>
                <strong>{model.displayName}</strong>
                <span className="model-details">
                  {gigabytes(model.sizeBytes)} · {model.source === "download" ? "Downloaded" : "Imported"}
                  {model.version ? ` · ${model.version}` : ""}
                </span>
              </span>
            </label>
          ))}
        </fieldset>
      )}
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
      <ModelSetup onReady={load} />
    </section>
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
