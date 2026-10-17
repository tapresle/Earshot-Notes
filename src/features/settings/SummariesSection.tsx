import { useEffect, useState } from "react";
import { api, errorMessage, type SummaryInstructions } from "../../lib/api";

/** Counts characters the way the Rust core does, so the count and the limit agree. */
function characters(text: string): number {
  return [...text.trim()].length;
}

/** The instructions summaries follow: the built-in default, or your own text in its place. */
export function SummariesSection() {
  const [instructions, setInstructions] = useState<SummaryInstructions | null>(null);
  // The text in the editor; null shows the default read-only instead.
  const [draft, setDraft] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .summaryInstructions()
      .then((loaded) => {
        setInstructions(loaded);
        setDraft(loaded.custom);
      })
      .catch((e) => setError(errorMessage(e)));
  }, []);

  async function save(text: string | null) {
    setSaving(true);
    setStatus(null);
    setError(null);
    try {
      const saved = await api.setSummaryInstructions(text);
      setInstructions(saved);
      setDraft(saved.custom);
      setStatus(
        saved.custom === null
          ? "Saved. Summaries use the default instructions."
          : "Saved. The next summary uses your instructions.",
      );
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  }

  function customize() {
    if (!instructions) return;
    setStatus(null);
    setDraft(instructions.default);
  }

  function cancel() {
    setStatus(null);
    setError(null);
    setDraft(null);
  }

  const count = draft === null ? 0 : characters(draft);
  const tooLong = instructions !== null && count > instructions.maxChars;

  return (
    <section className="settings-section" aria-labelledby="summaries-heading">
      <h2 id="summaries-heading">Summaries</h2>
      <p className="settings-help">
        Summaries follow these instructions. To fit summaries to your own meetings, replace them with your own.
        Summaries started before you save keep the instructions they started with.
      </p>
      {instructions === null && !error && <p className="empty-hint">Loading…</p>}
      {instructions !== null && (
        <>
          <p className="instructions-in-use">
            {instructions.custom === null ? "The default instructions are in use." : "Your instructions are in use."}
          </p>
          {draft === null ? (
            <>
              <pre className="instructions-text" tabIndex={0} aria-label="Default summary instructions">
                {instructions.default}
              </pre>
              <div className="vocabulary-actions">
                <button type="button" className="button" onClick={customize}>
                  Customize
                </button>
              </div>
            </>
          ) : (
            <>
              <textarea
                className="instructions-editor"
                aria-label="Your summary instructions"
                aria-describedby="instructions-count"
                value={draft}
                onChange={(e) => {
                  setStatus(null);
                  setDraft(e.target.value);
                }}
              />
              <p id="instructions-count" className={tooLong ? "instructions-count over" : "instructions-count"}>
                {count.toLocaleString("en-US")} / {instructions.maxChars.toLocaleString("en-US")} characters
              </p>
              <div className="vocabulary-actions">
                <button type="button" className="button" onClick={() => void save(draft)} disabled={saving}>
                  {saving ? "Saving…" : "Save"}
                </button>
                {instructions.custom === null ? (
                  <button type="button" className="button subtle" onClick={cancel} disabled={saving}>
                    Cancel
                  </button>
                ) : (
                  <button type="button" className="button subtle" onClick={() => void save(null)} disabled={saving}>
                    Reset to default
                  </button>
                )}
              </div>
            </>
          )}
          <p className="instructions-rules-label">Always added after the instructions:</p>
          <pre className="instructions-text" tabIndex={0} aria-label="Rules always added to summary instructions">
            {instructions.rules}
          </pre>
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
