import { useState } from "react";
import { api, errorMessage, type ExportReport } from "../../lib/api";

type Status =
  | { kind: "idle" }
  | { kind: "exporting" }
  | { kind: "done"; report: ExportReport; folder: string }
  | { kind: "error"; message: string };

function plural(count: number, word: string): string {
  return `${count} ${word}${count === 1 ? "" : "s"}`;
}

/** The sidebar footer: export the selected meetings, or all of them, as Markdown files. */
export function ExportBar({
  selectedIds,
  meetingCount,
  onClearSelection,
}: {
  selectedIds: number[];
  meetingCount: number;
  onClearSelection: () => void;
}) {
  const [status, setStatus] = useState<Status>({ kind: "idle" });

  async function exportTo(ids: number[] | null) {
    try {
      const folder = await api.pickFolder();
      if (!folder) return;
      setStatus({ kind: "exporting" });
      const report = await api.exportMeetings(ids, folder);
      setStatus({ kind: "done", report, folder });
    } catch (e) {
      setStatus({ kind: "error", message: errorMessage(e) });
    }
  }

  if (meetingCount === 0) return null;
  const busy = status.kind === "exporting";

  return (
    <div className="export-bar">
      {selectedIds.length > 1 && (
        <div className="selection-bar">
          <span>{plural(selectedIds.length, "meeting")} selected</span>
          <button type="button" className="button" onClick={() => void exportTo(selectedIds)} disabled={busy}>
            Export selected
          </button>
          <button type="button" className="button subtle" onClick={onClearSelection}>
            Clear
          </button>
        </div>
      )}
      <button type="button" className="button full-width" onClick={() => void exportTo(null)} disabled={busy}>
        {busy ? "Exporting…" : "Export all"}
      </button>
      <ExportStatus status={status} />
    </div>
  );
}

function ExportStatus({ status }: { status: Status }) {
  if (status.kind === "error") {
    return (
      <p className="field-error" role="alert">
        {status.message}
      </p>
    );
  }
  if (status.kind !== "done") return null;
  const { written, failed } = status.report;
  return (
    <div role="status" className="export-status">
      <p>
        Exported {plural(written.length, "meeting")} to <span className="selectable">{status.folder}</span>.
      </p>
      {failed.length > 0 && (
        <>
          <p className="field-error">Couldn't export {plural(failed.length, "meeting")}:</p>
          <ul>
            {failed.map((failure, i) => (
              <li key={i}>
                {failure.title}: {failure.message}
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

/** Exports one meeting to a file the user picks. */
export function ExportMeetingButton({ meetingId }: { meetingId: number }) {
  const [status, setStatus] = useState<{ kind: "idle" } | { kind: "done"; path: string } | { kind: "error"; message: string }>(
    { kind: "idle" },
  );

  async function exportMeeting() {
    try {
      const path = await api.pickSavePath(await api.exportFileName(meetingId));
      if (!path) return;
      setStatus({ kind: "done", path: await api.exportMeeting(meetingId, path) });
    } catch (e) {
      setStatus({ kind: "error", message: errorMessage(e) });
    }
  }

  return (
    <div className="export-meeting">
      <button type="button" className="button subtle" onClick={() => void exportMeeting()}>
        Export
      </button>
      {status.kind === "done" && (
        <span role="status" className="export-status">
          Saved to <span className="selectable">{status.path}</span>
        </span>
      )}
      {status.kind === "error" && (
        <span role="alert" className="field-error">
          {status.message}
        </span>
      )}
    </div>
  );
}
