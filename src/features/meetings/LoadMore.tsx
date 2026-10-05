import { useEffect, useRef } from "react";

interface LoadMoreProps {
  hasMore: boolean;
  loading: boolean;
  error: string | null;
  onLoadMore: () => void;
}

/**
 * The end of a paged list. Loads the next page when it scrolls into view, and
 * offers a button for the same thing. After an error it waits for the button.
 */
export function LoadMore({ hasMore, loading, error, onLoadMore }: LoadMoreProps) {
  const marker = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const target = marker.current;
    if (!hasMore || loading || error !== null || !target || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) onLoadMore();
    });
    observer.observe(target);
    return () => observer.disconnect();
  }, [hasMore, loading, error, onLoadMore]);

  if (!hasMore) return null;
  return (
    <div className="load-more" ref={marker}>
      {error !== null && (
        <p className="field-error" role="alert">
          Couldn't load more meetings. {error}
        </p>
      )}
      <button type="button" className="button subtle full-width" onClick={onLoadMore} disabled={loading}>
        {loading ? "Loading…" : "Show more meetings"}
      </button>
    </div>
  );
}
