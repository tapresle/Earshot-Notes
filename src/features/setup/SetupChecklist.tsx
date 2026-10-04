import { useEffect, useState } from "react";
import { api, errorMessage, type ModelRecord, type Permission, type PrivacyPane } from "../../lib/api";

/** Shown on first launch: permissions, a summary model, and a headphones tip. */
export function SetupChecklist({ onOpenSettings, onFinish }: { onOpenSettings: () => void; onFinish: () => void }) {
  const [mic, setMic] = useState<Permission | null>(null);
  const [model, setModel] = useState<ModelRecord | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .micPermission()
      .then(setMic)
      .catch((e) => setError(errorMessage(e)));
    api
      .summaryModel()
      .then((m) => setModel(m ?? null))
      .catch((e) => setError(errorMessage(e)));
  }, []);

  async function run(action: () => Promise<unknown>) {
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  const openSettings = (pane: PrivacyPane) => () => void run(() => api.openPrivacySettings(pane));

  return (
    <article className="setup">
      <h1>Get set up</h1>
      <p className="settings-help">Everything runs on this Mac. A few things to check before your first meeting:</p>
      <ol className="checklist">
        <li>
          <h2>Microphone</h2>
          {mic === "authorized" && <p className="check-done">Allowed</p>}
          {mic === "notDetermined" && (
            <>
              <p>Needed to transcribe your side of the conversation.</p>
              <button
                type="button"
                className="button primary"
                onClick={() => void run(async () => setMic(await api.requestMicPermission()))}
              >
                Allow microphone
              </button>
            </>
          )}
          {(mic === "denied" || mic === "restricted") && (
            <>
              <p className="field-warning">
                Microphone access is off. Turn on Earshot in Privacy &amp; Security &gt; Microphone.
              </p>
              <button type="button" className="button" onClick={openSettings("microphone")}>
                Open Microphone settings
              </button>
            </>
          )}
        </li>
        <li>
          <h2>The other side of your calls</h2>
          <p>
            macOS asks the first time you record. If you decline, the other side won't be transcribed until you allow
            Earshot in Privacy &amp; Security &gt; Screen &amp; System Audio Recording.
          </p>
          <button type="button" className="button" onClick={openSettings("systemAudio")}>
            Open audio recording settings
          </button>
        </li>
        <li>
          <h2>Summaries</h2>
          {model === undefined ? null : model ? (
            <p className="check-done">Ready, using {model.displayName}</p>
          ) : (
            <>
              <p>Summaries need a local model, a one-time download of about 3 GB. Transcription works without it.</p>
              <button type="button" className="button" onClick={onOpenSettings}>
                Set up summaries
              </button>
            </>
          )}
        </li>
        <li>
          <h2>Headphones</h2>
          <p>Use headphones in calls. With speakers, your microphone also picks up the other side.</p>
        </li>
      </ol>
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
      <button type="button" className="button primary" onClick={() => void run(async () => {
        await api.setSetupComplete(true);
        onFinish();
      })}>
        Finish setup
      </button>
    </article>
  );
}
