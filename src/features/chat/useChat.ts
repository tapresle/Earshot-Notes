import { useCallback, useEffect, useRef, useState } from "react";
import { api, type ChatMessage, type ChatStage, errorMessage } from "../../lib/api";

export type ChatRun =
  | { kind: "idle" }
  /** `question` is empty when the answer was already running when the meeting opened. */
  | { kind: "running"; question: string; stage: ChatStage; text: string }
  | { kind: "failed"; question: string; message: string; cancelled: boolean };

export interface ChatState {
  loading: boolean;
  loadError: string | null;
  messages: ChatMessage[];
  run: ChatRun;
  /** The question being typed. A cancelled or failed question comes back here. */
  draft: string;
  setDraft: (draft: string) => void;
  /** Asks a question. Resolves to an error message, or null once it's queued. */
  ask: (question: string) => Promise<string | null>;
  cancel: () => void;
  reload: () => void;
}

/** The meeting's saved conversation and any answer being written. */
export function useChat(meetingId: number): ChatState {
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [run, setRun] = useState<ChatRun>({ kind: "idle" });
  const [draft, setDraft] = useState("");
  const [attempt, setAttempt] = useState(0);
  // The question being answered, for the failure handler to give back.
  const asked = useRef("");

  useEffect(() => {
    let cancelled = false;
    Promise.all([api.chatMessages(meetingId), api.activeChat()])
      .then(([saved, active]) => {
        if (cancelled) return;
        setMessages(saved);
        if (active === meetingId) {
          setRun((current) =>
            current.kind === "running" ? current : { kind: "running", question: "", stage: { stage: "loading" }, text: "" },
          );
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
      api.onChatProgress(({ meetingId: id, ...stage }) => {
        if (id !== meetingId) return;
        setRun((current) =>
          current.kind === "running" ? { ...current, stage } : { kind: "running", question: "", stage, text: "" },
        );
      }),
      api.onChatText(({ meetingId: id, text }) => {
        if (id !== meetingId) return;
        setRun((current) => (current.kind === "running" ? { ...current, text: current.text + text } : current));
      }),
      api.onChatDone(({ meetingId: id, question, answer }) => {
        if (id !== meetingId) return;
        setMessages((current) => [...current, question, answer]);
        asked.current = "";
        setRun({ kind: "idle" });
      }),
      api.onChatFailed(({ meetingId: id, message, cancelled }) => {
        if (id !== meetingId) return;
        const question = asked.current;
        asked.current = "";
        setDraft((typed) => typed || question);
        setRun({ kind: "failed", question, message, cancelled });
      }),
    ];
    return () => {
      for (const unlisten of unlisteners) unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [meetingId]);

  const ask = useCallback(
    async (question: string) => {
      asked.current = question;
      setRun({ kind: "running", question, stage: { stage: "loading" }, text: "" });
      try {
        await api.askMeeting(meetingId, question);
        return null;
      } catch (error) {
        setRun({ kind: "idle" });
        return errorMessage(error);
      }
    },
    [meetingId],
  );

  const cancel = useCallback(() => {
    api.cancelChat().catch((error) => console.error("couldn't cancel the answer", error));
  }, []);

  return {
    loading,
    loadError,
    messages,
    run,
    draft,
    setDraft,
    ask,
    cancel,
    reload: () => {
      setLoading(true);
      setAttempt((n) => n + 1);
    },
  };
}
