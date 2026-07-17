import { useEffect, useRef } from 'react';

/** Fire `callback` every `intervalMs` while `enabled` AND the document is
 *  visible. Hiding the tab pauses the interval; returning to it resumes with
 *  an immediate tick (the data is stale by definition after a pause). The
 *  callback is kept in a ref so a fresh closure never restarts the timer. */
export function useVisibilityPoll(callback: () => void, intervalMs: number, enabled: boolean) {
  const cbRef = useRef(callback);
  cbRef.current = callback;

  useEffect(() => {
    if (!enabled) return;
    let timer: number | null = null;

    const start = () => {
      if (timer == null) {
        timer = window.setInterval(() => cbRef.current(), intervalMs);
      }
    };
    const stop = () => {
      if (timer != null) {
        window.clearInterval(timer);
        timer = null;
      }
    };
    const onVisibility = () => {
      if (document.hidden) {
        stop();
      } else {
        cbRef.current();
        start();
      }
    };

    if (!document.hidden) start();
    document.addEventListener('visibilitychange', onVisibility);
    return () => {
      stop();
      document.removeEventListener('visibilitychange', onVisibility);
    };
  }, [intervalMs, enabled]);
}
