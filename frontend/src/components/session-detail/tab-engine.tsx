import { useEffect, useRef } from 'react';
import type { ReactNode } from 'react';
import { useContent } from '@/i18n';
import type { SessionDetail } from '@/lib/api/types';
import { FadeSwap } from '@/components/ui/motion';
import { Note, SectionLabel, Spinner } from './parts';
import { ObserveView } from './observe-view';
import type { ObserveState } from './observe-state';
import { fallbackRecovery, isRuntimeLive } from './recovery-state';

/**
 * Engine tab: the session's LIVE runtime observation — the engine's queues,
 * in-flight work and durable deliveries, read out of the running pod.
 *
 * Split out of Status (#5841) because the two answer different questions.
 * Status answers "where is this session in its lifecycle", which is derived from
 * data already in hand; this answers "what is the engine doing right now", which
 * costs a pod exec that can take the better part of a minute. Mixing them meant
 * every reader who opened Status paid for a read they had not asked for.
 *
 * Opening this tab IS the request for that read, so it fires on activation —
 * once, and only while the runtime is positively live.
 */
export function TabEngine({
  session,
  observe,
  onLoadObserve,
}: {
  session: SessionDetail;
  observe: ObserveState;
  onLoadObserve: () => void;
}) {
  const t = useContent().dashboard.detail;
  const recovery = session.recovery ?? fallbackRecovery(session);
  // The observe read pod-execs INTO the running pod, so it is only meaningful —
  // and only permitted — while the runtime is positively live.
  const isLive = isRuntimeLive(session);

  // Hard gate: never let the observe fetch fire unless the pod is live, even if
  // a stray caller reaches the handler.
  const handleLoadObserve = () => {
    if (isLive) onLoadObserve();
  };

  // Fire at most ONCE per mount. The `idle` guard alone would already do it
  // (loadObserve sets `loading` synchronously), but the ref makes the intent
  // explicit and survives an unstable inline callback from the host. Crucially
  // it never re-fires from an ERROR state: a minute-long pod exec looping on
  // failure is the worst outcome this surface could produce — the retry button
  // below is the deliberate, human way back.
  const requested = useRef(false);
  useEffect(() => {
    if (!isLive || requested.current || observe.status !== 'idle') return;
    requested.current = true;
    onLoadObserve();
  }, [isLive, observe.status, onLoadObserve]);

  return (
    <div className="flex flex-col gap-5">
      <section className="flex flex-col gap-2">
        <SectionLabel>{t.liveEngine}</SectionLabel>
        {isLive ? (
          // Crossfade the observe states keyed on `status`: the fetched engine
          // snapshot slides in under the label as loading resolves to loaded,
          // rather than popping the panel in. Instant under reduced motion.
          <FadeSwap k={observe.status}>{renderObserve()}</FadeSwap>
        ) : (
          // Paused/idle: the pod is gone, so there is nothing to observe. Explain
          // it calmly instead of offering a fetch that would only error.
          <Note>
            {recovery.state === 'recovering'
              ? t.liveEngineRecovering
              : recovery.state === 'idle'
                ? t.liveEnginePaused
                : t.liveEngineNotLive}
          </Note>
        )}
      </section>
    </div>
  );

  function renderObserve(): ReactNode {
    switch (observe.status) {
      case 'idle':
        // Reachable only if the auto-load has not run yet (or a host passed no
        // usable callback), so the manual affordance stays as the fallback.
        return (
          <button
            type="button"
            onClick={handleLoadObserve}
            className="self-start font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-dim transition-[color,border-color,box-shadow] duration-150 hover:text-fg hover:border-line-2 hover:shadow-glow-amber cursor-pointer"
          >
            {t.liveEngine}
          </button>
        );
      case 'loading':
        return (
          <div className="flex flex-col gap-1.5">
            <span className="inline-flex items-center gap-2 font-mono text-[11.5px] text-dim">
              <Spinner />
              {t.liveEngineLoading}
            </span>
            <Note>{t.liveEngineSlow}</Note>
          </div>
        );
      case 'error': {
        // Explain the failure: 409 == no durable delivery store to observe;
        // anything else is a transient/defensive fallback (the section is
        // already gated on live, so this is rarely reached). A 409 will not
        // recover on retry, so only offer the retry for the transient case.
        const noStore = observe.httpStatus === 409;
        return (
          <div className="flex flex-col items-start gap-2">
            <p className="text-[12.5px] text-red">
              {noStore ? t.liveEngineErrorNoStore : t.liveEngineNotLive}
            </p>
            {!noStore && (
              <button
                type="button"
                onClick={handleLoadObserve}
                className="font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-dim transition-[color,border-color,box-shadow] duration-150 hover:text-fg hover:border-line-2 hover:shadow-glow-amber cursor-pointer"
              >
                {t.logsRefresh}
              </button>
            )}
          </div>
        );
      }
      case 'loaded':
        return <ObserveView snapshot={observe.snapshot} />;
    }
  }
}
