import { useContent, useLang } from '@/i18n';
import { cn } from '@/lib/utils';
import { formatIsoSgt } from '@/lib/format';
import { FIELD_INPUT } from '@/components/ui/field';
import type { LogRun } from '@/lib/api/logs';
import { SectionLabel } from './parts';

// The Logs-tab run picker: a substrate session is served by a sequence of pod
// incarnations ("runs"); this lets the reader pick which incarnation's bundle
// to view. Timestamps render in SGT (Asia/Singapore) via `formatIsoSgt` — the
// product requirement for these windows.

/** A run's decoded label: its SGT start–end window, a "running" line for the
 *  open current incarnation (no `ended_at`), or a compact "Latest logs" for a
 *  legacy synthetic run whose `started_at` is empty (`formatIsoSgt` returns
 *  null for an empty/invalid instant). */
interface RunLabel {
  text: string;
  running: boolean;
}

function useRunLabeler(): (run: LogRun) => RunLabel {
  const t = useContent().dashboard.detail;
  const { lang } = useLang();
  return (run: LogRun): RunLabel => {
    const start = formatIsoSgt(run.started_at, lang);
    // Empty/unparseable start ⇒ the legacy synthetic "latest" run: no window.
    if (!start) return { text: t.runLatest, running: false };
    const end = run.ended_at ? formatIsoSgt(run.ended_at, lang) : null;
    // No end ⇒ the current incarnation is still running.
    if (!end) return { text: t.runRunning.replace('{start}', start), running: true };
    return { text: `${start} – ${end}`, running: false };
  };
}

/** Live "running" marker — a small dot that softly blinks; `.anim-dot-blink` is
 *  disabled under prefers-reduced-motion (leaving a static dot), so the running
 *  state is never conveyed by motion alone (the "· running" text carries it). */
function RunningDot() {
  return (
    <span
      aria-hidden="true"
      className="anim-dot-blink w-1.5 h-1.5 rounded-full bg-green flex-none"
    />
  );
}

/** Run picker rendered above the manifest/file view. With exactly one run it
 *  collapses to a compact static label (no control); with several it is a
 *  token-styled `<select>`, newest first, adorned with a live dot when the
 *  selected run is still running. */
export function RunPicker({
  runs,
  selectedRun,
  onSelect,
}: {
  /** Always non-empty — the caller renders nothing (latest-only fallback) when
   *  there are no runs. */
  runs: LogRun[];
  selectedRun: string | null;
  onSelect: (runId: string) => void;
}) {
  const t = useContent().dashboard.detail;
  const label = useRunLabeler();

  // Exactly one run: a compact static label, not a control.
  if (runs.length === 1) {
    const only = runs[0]!;
    const { text, running } = label(only);
    return (
      <div className="flex items-center gap-2 flex-wrap">
        <SectionLabel>{t.runPicker}</SectionLabel>
        <span className="inline-flex items-center gap-1.5 font-mono text-[11.5px] text-dim">
          {running && <RunningDot />}
          {text}
        </span>
      </div>
    );
  }

  const current = runs.find((r) => r.run_id === selectedRun) ?? runs[0]!;
  const currentRunning = label(current).running;

  return (
    <div className="flex items-center gap-2 flex-wrap">
      <SectionLabel>{t.runPicker}</SectionLabel>
      <div className="inline-flex items-center gap-1.5">
        {currentRunning && <RunningDot />}
        <select
          aria-label={t.runPicker}
          value={selectedRun ?? current.run_id}
          onChange={(e) => onSelect(e.target.value)}
          className={cn(FIELD_INPUT, 'w-auto cursor-pointer py-1.5 text-[12px] font-mono')}
        >
          {runs.map((run) => (
            <option key={run.run_id} value={run.run_id}>
              {label(run).text}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}
