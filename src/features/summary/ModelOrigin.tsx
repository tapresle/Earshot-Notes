import { api, errorMessage, type HuggingFaceSource } from "../../lib/api";

/** Exactly which file a model is on Hugging Face, with a link to its page. */
export function ModelOrigin({
  modelId,
  displayName,
  source,
  onError,
}: {
  modelId: string;
  displayName: string;
  source: HuggingFaceSource;
  onError: (message: string) => void;
}) {
  return (
    <p className="model-origin">
      <code className="selectable">
        {source.repo}/{source.fileName}
      </code>{" "}
      at revision <code className="selectable">{source.revision.slice(0, 7)}</code> ·{" "}
      <button
        type="button"
        className="link-button"
        aria-label={`Hugging Face page for ${displayName}`}
        onClick={() => void api.openModelPage(modelId).catch((e) => onError(errorMessage(e)))}
      >
        Hugging Face page
      </button>
    </p>
  );
}
