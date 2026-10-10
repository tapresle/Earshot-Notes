import { api, errorMessage, type HuggingFaceSource } from "../../lib/api";

/**
 * Exactly which file a model is on Hugging Face: the repository, which links to its page, then the quantization (or
 * the variant, in a repository that holds several models) and the pinned revision.
 */
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
      <button
        type="button"
        className="link-button"
        title={`Open ${displayName} on Hugging Face`}
        onClick={() => void api.openModelPage(modelId).catch((e) => onError(errorMessage(e)))}
      >
        {source.repo}
      </button>{" "}
      - <code className="selectable">{source.variant ?? source.quantization}</code> -{" "}
      <code className="selectable">{source.revision.slice(0, 7)}</code>
    </p>
  );
}
