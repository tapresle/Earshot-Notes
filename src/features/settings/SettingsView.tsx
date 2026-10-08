import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  errorMessage,
  isCommandError,
  type InputDevice,
  type InstalledModels,
  type LaunchAtLogin,
  type InstalledModel as InstalledModelInfo,
  type ModelRecord,
  type Sensitivity,
  type Source,
  type SpeechSensitivity,
  type TranscriptionModel,
  type UpdateCheck,
  type VocabularyTerm,
} from "../../lib/api";
import { ExportAllButton } from "../export/ExportBar";
import { ModelOrigin } from "../summary/ModelOrigin";
import { Markdown } from "../summary/Markdown";
import { ModelSetup } from "../summary/ModelSetup";
import { missingTerms, SOFTWARE_TERMS } from "./softwareTerms";

export function SettingsView({ recording, onShowSetup }: { recording: boolean; onShowSetup: () => void }) {
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
      <StartupSection />
      <MicrophoneSection />
      <SpeechDetectionSection />
      <VocabularySection />
      <AudioRecordingsSection recording={recording} />
      <SummaryModelSection />
      <TranscriptionSection />
      <UpdatesSection />
      <section className="settings-section" aria-labelledby="export-heading">
        <h2 id="export-heading">Export</h2>
        <p className="settings-help">Saves every meeting as a Markdown file in a folder you choose.</p>
        <ExportAllButton />
      </section>
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

function StartupSection() {
  const [state, setState] = useState<LaunchAtLogin | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .launchAtLogin()
      .then(setState)
      .catch((e) => setError(errorMessage(e)));
  }, []);

  async function toggle(enabled: boolean) {
    setError(null);
    try {
      setState(await api.setLaunchAtLogin(enabled));
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <section className="settings-section" aria-labelledby="startup-heading">
      <h2 id="startup-heading">Startup</h2>
      {state && (
        <label className="settings-row">
          <input
            type="checkbox"
            checked={state.enabled}
            disabled={!state.available}
            onChange={(e) => void toggle(e.target.checked)}
          />
          Open at login
        </label>
      )}
      <p className="settings-help">
        Opens Earshot Notes in the menu bar when you log in. Closing the window keeps it running there.
      </p>
      {state && !state.available && (
        <p className="field-warning">Opening at login isn't available on this system.</p>
      )}
      {state?.needsApproval && (
        <p className="field-warning">
          Allow Earshot Notes in System Settings &gt; General &gt; Login Items to finish turning this on.
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

const SENSITIVITY_LEVELS: { value: Sensitivity; label: string }[] = [
  { value: "low", label: "Low" },
  { value: "normal", label: "Normal" },
  { value: "high", label: "High" },
];

const SENSITIVITY_SOURCES: { source: Source; label: string }[] = [
  { source: "me", label: "My voice" },
  { source: "them", label: "Meeting Participants" },
];

function SpeechDetectionSection() {
  const [levels, setLevels] = useState<SpeechSensitivity | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    api
      .speechSensitivity()
      .then(setLevels)
      .catch((e) => setError(errorMessage(e)));
  }, []);

  async function choose(source: Source, level: Sensitivity) {
    setSaved(false);
    setError(null);
    try {
      await api.setSpeechSensitivity(source, level);
      setLevels((current) => (current ? { ...current, [source]: level } : current));
      setSaved(true);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <section className="settings-section" aria-labelledby="speech-detection-heading">
      <h2 id="speech-detection-heading">Speech detection</h2>
      <p className="settings-help">
        Raise this if quiet speech is missed. Lower it if background noise is transcribed. Changes apply to the next
        recording.
      </p>
      {levels === null && !error && <p className="empty-hint">Loading…</p>}
      {levels !== null && (
        <div className="settings-grid">
          {SENSITIVITY_SOURCES.map(({ source, label }) => (
            <Fragment key={source}>
              <label htmlFor={`sensitivity-${source}`}>{label}</label>
              <select
                id={`sensitivity-${source}`}
                value={levels[source]}
                onChange={(e) => void choose(source, e.target.value as Sensitivity)}
              >
                {SENSITIVITY_LEVELS.map(({ value, label }) => (
                  <option key={value} value={value}>
                    {label}
                  </option>
                ))}
              </select>
            </Fragment>
          ))}
        </div>
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

function fileSize(bytes: number): string {
  if (bytes >= 1e9) return gigabytes(bytes);
  if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`;
  return "less than 1 MB";
}

function AudioRecordingsSection({ recording }: { recording: boolean }) {
  const [keep, setKeep] = useState<boolean | null>(null);
  const [size, setSize] = useState<number | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const deleteRef = useRef<HTMLButtonElement>(null);
  const opened = useRef(false);

  useEffect(() => {
    api
      .keepAudio()
      .then(setKeep)
      .catch((e) => setError(errorMessage(e)));
  }, []);

  // A recording that just ended may have added files.
  useEffect(() => {
    api
      .recordingsSize()
      .then(setSize)
      .catch((e) => setError(errorMessage(e)));
  }, [recording]);

  // Focus Cancel when the confirmation opens, and return to the delete button when it closes.
  useEffect(() => {
    if (confirming) {
      opened.current = true;
      cancelRef.current?.focus();
    } else if (opened.current) {
      deleteRef.current?.focus();
    }
  }, [confirming]);

  async function toggle(enabled: boolean) {
    setError(null);
    setStatus(null);
    try {
      await api.setKeepAudio(enabled);
      setKeep(enabled);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function deleteAll() {
    setError(null);
    setDeleting(true);
    try {
      setSize(await api.deleteAllRecordings());
      setStatus("Deleted all recordings.");
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setDeleting(false);
      setConfirming(false);
    }
  }

  async function show() {
    setError(null);
    try {
      await api.showRecordings();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  const empty = size === 0;

  return (
    <section className="settings-section" aria-labelledby="audio-recordings-heading">
      <h2 id="audio-recordings-heading">Audio recordings</h2>
      {keep !== null && (
        <label className="settings-row">
          <input type="checkbox" checked={keep} onChange={(e) => void toggle(e.target.checked)} />
          Keep audio recordings
        </label>
      )}
      <p className="settings-help">Saves what each recording hears, for testing and review. Recordings stay on this Mac.</p>
      {size !== null && (
        <div className="settings-row">
          {confirming ? (
            <div className="model-remove-confirm" role="group" aria-label="Delete all recordings">
              <span>Delete every kept recording and free {fileSize(size)}? This can't be undone.</span>
              <button type="button" className="button danger" onClick={() => void deleteAll()} disabled={deleting}>
                {deleting ? "Deleting…" : "Delete"}
              </button>
              <button
                type="button"
                className="button subtle"
                ref={cancelRef}
                onClick={() => setConfirming(false)}
                disabled={deleting}
              >
                Cancel
              </button>
            </div>
          ) : (
            <>
              <button
                type="button"
                className="button danger"
                ref={deleteRef}
                onClick={() => {
                  setStatus(null);
                  setConfirming(true);
                }}
                disabled={recording || empty}
                aria-describedby={recording && !empty ? "recordings-busy" : undefined}
              >
                {empty ? "No recordings to delete" : `Delete all recordings (${fileSize(size)})`}
              </button>
              <button type="button" className="button subtle" onClick={() => void show()}>
                Show in Finder
              </button>
            </>
          )}
        </div>
      )}
      {recording && !empty && !confirming && (
        <p className="settings-help" id="recordings-busy">
          Stop recording to delete recordings.
        </p>
      )}
      {status && (
        <p className="settings-saved" role="status">
          {status}
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
  model: InstalledModelInfo;
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
            {model.version && !model.huggingFace ? ` · ${model.version}` : ""}
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
      {model.huggingFace && (
        <ModelOrigin
          modelId={model.id}
          displayName={model.displayName}
          source={model.huggingFace}
          onError={onError}
        />
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

function UpdatesSection() {
  const [enabled, setEnabled] = useState(false);
  const [checking, setChecking] = useState(false);
  const [found, setFound] = useState<UpdateCheck | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .updateCheckEnabled()
      .then(setEnabled)
      .catch((e) => setError(errorMessage(e)));
  }, []);

  async function check() {
    setChecking(true);
    setError(null);
    setFound(null);
    try {
      setFound(await api.checkForUpdate());
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setChecking(false);
    }
  }

  async function openPage(version: string) {
    setError(null);
    try {
      await api.openReleasePage(version);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  if (!enabled && !error) return null;

  return (
    <section className="settings-section" aria-labelledby="updates-heading">
      <h2 id="updates-heading">Updates</h2>
      {enabled && (
        <>
          <p className="settings-help">
            Checks GitHub for a newer release. Earshot Notes contacts GitHub only when you press the button.
          </p>
          <button type="button" className="button subtle" onClick={() => void check()} disabled={checking}>
            {checking ? "Checking…" : "Check for updates"}
          </button>
        </>
      )}
      {found && !found.newer && <p role="status">You have the latest version, {found.current}.</p>}
      {found?.newer && (
        <div className="update-card">
          <p role="status">
            Earshot Notes {found.latest} is available. You have {found.current}.
          </p>
          <button type="button" className="button" onClick={() => void openPage(found.latest)}>
            Open download page
          </button>
          {found.notes && <Markdown source={found.notes} />}
        </div>
      )}
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}

interface VocabularyRow {
  key: number;
  term: string;
  /** Mishearings, separated by commas. */
  heardAs: string;
}

function VocabularySection() {
  const [rows, setRows] = useState<VocabularyRow[] | null>(null);
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const nextKey = useRef(0);

  const toRows = useCallback(
    (terms: VocabularyTerm[]) =>
      terms.map((t) => ({ key: nextKey.current++, term: t.term, heardAs: t.heardAs.join(", ") })),
    [],
  );

  useEffect(() => {
    api
      .vocabulary()
      .then((terms) => setRows(toRows(terms)))
      .catch((e) => setError(errorMessage(e)));
  }, [toRows]);

  function change(key: number, field: "term" | "heardAs", value: string) {
    setStatus(null);
    setRows((current) => current && current.map((row) => (row.key === key ? { ...row, [field]: value } : row)));
  }

  function add() {
    setStatus(null);
    setRows((current) => [...(current ?? []), { key: nextKey.current++, term: "", heardAs: "" }]);
  }

  function remove(key: number) {
    setStatus(null);
    setRows((current) => current && current.filter((row) => row.key !== key));
  }

  function addSoftwareTerms() {
    if (!rows) return;
    const added = missingTerms(
      rows.map((row) => row.term),
      SOFTWARE_TERMS,
    );
    if (added.length === 0) {
      setStatus("Every software term is already in your vocabulary.");
      return;
    }
    setRows([...rows, ...added.map((term) => ({ key: nextKey.current++, term, heardAs: "" }))]);
    setStatus(
      `Added ${added.length} software ${added.length === 1 ? "term" : "terms"}. Save the vocabulary to keep them.`,
    );
  }

  async function save() {
    if (!rows) return;
    setSaving(true);
    setStatus(null);
    setError(null);
    try {
      const terms = rows
        .filter((row) => row.term.trim() || row.heardAs.trim())
        .map((row) => ({ term: row.term, heardAs: row.heardAs.split(",") }));
      const saved = await api.setVocabulary(terms);
      setRows(toRows(saved.terms));
      setStatus(
        saved.updated === 0
          ? "Saved"
          : `Saved. Corrected ${saved.updated} transcript ${saved.updated === 1 ? "line" : "lines"}.`,
      );
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="settings-section" aria-labelledby="vocabulary-heading">
      <h2 id="vocabulary-heading">Vocabulary</h2>
      <p className="settings-help">
        Names your meetings use, such as products, customers, and colleagues. Transcription listens for them from
        the next recording, and summaries spell them as written. Under <strong>Heard as</strong>, list what
        transcription writes instead, separated by commas. Saving fixes those words in every transcript, and
        removing them restores the original. If your meetings are about software, <strong>Add software terms</strong>{" "}
        adds technical terms that transcription often mishears, such as &ldquo;async&rdquo; and &ldquo;JSON&rdquo;.
      </p>
      {rows === null && !error && <p className="empty-hint">Loading…</p>}
      {rows !== null && (
        <>
          {rows.length > 0 && (
            <div className="vocabulary-grid">
              <span className="vocabulary-heading" aria-hidden="true">
                Term
              </span>
              <span className="vocabulary-heading" aria-hidden="true">
                Heard as
              </span>
              <span aria-hidden="true" />
              {rows.map((row, index) => (
                <Fragment key={row.key}>
                  <input
                    className="vocabulary-input"
                    aria-label={`Term ${index + 1}`}
                    value={row.term}
                    onChange={(e) => change(row.key, "term", e.target.value)}
                  />
                  <input
                    className="vocabulary-input"
                    aria-label={`Heard as, term ${index + 1}`}
                    value={row.heardAs}
                    onChange={(e) => change(row.key, "heardAs", e.target.value)}
                  />
                  <button
                    type="button"
                    className="button subtle"
                    aria-label={`Remove term ${index + 1}`}
                    onClick={() => remove(row.key)}
                  >
                    Remove
                  </button>
                </Fragment>
              ))}
            </div>
          )}
          <div className="vocabulary-actions">
            <button type="button" className="button subtle" onClick={add}>
              Add term
            </button>
            <button type="button" className="button subtle" onClick={addSoftwareTerms}>
              Add software terms
            </button>
            <button type="button" className="button" onClick={() => void save()} disabled={saving}>
              {saving ? "Saving…" : "Save vocabulary"}
            </button>
          </div>
        </>
      )}
      {status && (
        <p className="settings-saved" role="status">
          {status}
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

