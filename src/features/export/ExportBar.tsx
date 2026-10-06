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

/** Exports meetings to a folder the user picks, and tracks how it went. */
function useFolderExport() {
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

  return { status, exportTo, busy: status.kind === "exporting" };
}

/** The sidebar footer while several meetings are selected: export them as Markdown files. */
export function ExportSelectionBar({
  selectedIds,
  onClearSelection,
}: {
  selectedIds: number[];
  onClearSelection: () => void;
}) {
  const { status, exportTo, busy } = useFolderExport();
  if (selectedIds.length < 2 && status.kind === "idle") return null;

  return (
    <div className="export-bar">
      {selectedIds.length > 1 && (
        <div className="selection-bar">
          <span>{plural(selectedIds.length, "meeting")} selected</span>
          <button type="button" className="button" onClick={() => void exportTo(selectedIds)} disabled={busy}>
            {busy ? "Exporting…" : "Export selected"}
          </button>
          <button type="button" className="button subtle" onClick={onClearSelection}>
            Clear
          </button>
        </div>
      )}
      <ExportStatus status={status} />
    </div>
  );
}

/** Exports every meeting, whatever the sidebar shows. */
export function ExportAllButton() {
  const { status, exportTo, busy } = useFolderExport();

  return (
    <div className="export-all">
      <button type="button" className="button" onClick={() => void exportTo(null)} disabled={busy}>
        {busy ? "Exporting…" : "Export all meetings"}
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
  if (written.length === 0 && failed.length === 0) {
    return (
      <p role="status" className="export-status">
        There are no meetings to export yet.
      </p>
    );
  }
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
