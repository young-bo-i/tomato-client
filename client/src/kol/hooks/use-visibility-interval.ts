import { useEffect } from "react";

/**
 * Sets up a setInterval that automatically pauses when the document is hidden
 * and resumes (with an immediate call) when it becomes visible again.
 *
 * Does NOT call `fn` on initial mount — the component should handle the first
 * fetch via its own useEffect. On visibility restore the fn IS called
 * immediately so stale data doesn't sit on screen after the user comes back.
 *
 * @param fn      Callback to call on each tick. Must be stable (useCallback).
 * @param ms      Interval in milliseconds.
 * @param enabled Pass false to disable entirely (e.g. an autoRefresh toggle).
 */
export function useVisibilityInterval(
  fn: () => void,
  ms: number,
  enabled = true,
): void {
  useEffect(() => {
    if (!enabled) return;

    let timer: ReturnType<typeof setInterval> | null = null;

    const start = (callImmediate: boolean) => {
      if (timer !== null) return;
      if (callImmediate) fn();
      timer = setInterval(fn, ms);
    };
    const stop = () => {
      if (timer !== null) {
        clearInterval(timer);
        timer = null;
      }
    };
    const onVisibility = () => {
      if (document.visibilityState === "visible") start(true);
      else stop();
    };

    // Start interval without immediate call — component's own useEffect
    // handles the first fetch so we don't double-load on mount.
    start(false);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      stop();
    };
  }, [fn, ms, enabled]);
}
