import { useCallback, useEffect, useEffectEvent, useState } from "react";
import { api, errorMessage, type ModelRecord, type Summary, type SummaryStage } from "../../lib/api";

export type SummaryRun =
  | { kind: "idle" }
  | { kind: "running"; stage: SummaryStage; text: string }
  | { kind: "failed"; message: string; cancelled: boolean };

export interface SummaryState {
  loading: boolean;
  loadError: string | null;
  summary: Summary | null;
  model: ModelRecord | null;
  run: SummaryRun;
  /** Starts a summary. Resolves to an error message, or null on success. */
  start: () => Promise<string | null>;
  cancel: () => void;
  setModel: (model: ModelRecord) => void;
  reload: () => void;
}

/**
 * The meeting's latest summary, the selected model, and any summary in progress.
 * `onStartedElsewhere` runs when a summary this view didn't start begins or
 * fails, such as the one that starts when you stop recording.
 */
export function useSummary(meetingId: number, onStartedElsewhere: () => void): SummaryState {
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [summary, setSummary] = useState<Summary | null>(null);
  const [model, setModel] = useState<ModelRecord | null>(null);
  const [run, setRun] = useState<SummaryRun>({ kind: "idle" });
  const [attempt, setAttempt] = useState(0);
  // Summarize marks its own run as running before the backend reports on it,
  // so progress or a failure while idle comes from a run started elsewhere.
  const noticeElsewhere = useEffectEvent(() => {
    if (run.kind !== "running") onStartedElsewhere();
  });

  useEffect(() => {
    let cancelled = false;
    Promise.all([api.latestSummary(meetingId), api.selectedModel(), api.activeSummary()])
      .then(([latest, selected, active]) => {
        if (cancelled) return;
        setSummary(latest ?? null);
        setModel(selected ?? null);
        if (active === meetingId) {
          setRun((current) => (current.kind === "running" ? current : { kind: "running", stage: { stage: "loading" }, text: "" }));
        }
        setLoadError(null);
      })
      .catch((error) => !cancelled && setLoadError(errorMessage(error)))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [meetingId, attempt]);

  useEffect(() => {
    const unlisteners: Promise<() => void>[] = [
      api.onSummaryProgress(({ meetingId: id, ...stage }) => {
        if (id !== meetingId) return;
        noticeElsewhere();
        setRun((current) => ({ kind: "running", stage, text: current.kind === "running" ? current.text : "" }));
      }),
      api.onSummaryText(({ meetingId: id, text }) => {
        if (id !== meetingId) return;
        setRun((current) => (current.kind === "running" ? { ...current, text: current.text + text } : current));
      }),
      api.onSummaryDone(({ meetingId: id, summary: done }) => {
        if (id !== meetingId) return;
        setSummary(done);
        setRun({ kind: "idle" });
      }),
      api.onSummaryFailed(({ meetingId: id, message, cancelled }) => {
        if (id !== meetingId) return;
        noticeElsewhere();
        setRun({ kind: "failed", message, cancelled });
      }),
    ];
    return () => {
      for (const unlisten of unlisteners) unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [meetingId]);

  const start = useCallback(async () => {
    setRun({ kind: "running", stage: { stage: "loading" }, text: "" });
    try {
      await api.summarizeMeeting(meetingId);
      return null;
    } catch (error) {
      setRun({ kind: "idle" });
      return errorMessage(error);
    }
  }, [meetingId]);

  const cancel = useCallback(() => {
    api.cancelSummary().catch((error) => console.error("couldn't cancel the summary", error));
  }, []);

  return {
    loading,
    loadError,
    summary,
    model,
    run,
    start,
    cancel,
    setModel,
    reload: () => {
      setLoading(true);
      setAttempt((n) => n + 1);
    },
  };
}
