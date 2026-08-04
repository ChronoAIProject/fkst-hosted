import { useContent } from '@/i18n';
import {
  METHODS,
  OPERATION_CATALOG,
  OUTCOMES,
  RECORD_KINDS,
  STATUS_CLASSES,
} from '@/lib/api/operations';
import type { ActivityFilters, WindowProblem } from '@/lib/operations/state';
import {
  DEFAULT_ACTIVITY_FILTERS,
  TIME_PRESETS,
  parseLogin,
  parsePositiveInt,
  parseRepoFullName,
  parseRequestId,
  parseSessionId,
  parseStatusCode,
} from '@/lib/operations/state';
import {
  InstantFilter,
  ResetFiltersButton,
  SelectFilter,
  TextFilter,
} from './filter-controls';
import { RefreshButton } from './parts';

/**
 * The Activity view's filter toolbar.
 *
 * The actor controls are the security-shaped part: they are rendered ONLY in the
 * global scope. In a personal scope the server injects the caller's own verified
 * identity into the predicate, so an actor filter is not a narrower question —
 * it is a different, forbidden one, and offering the control would invite a
 * `403` the user has no way to interpret. Hiding it is a courtesy, not the
 * boundary; the backend refuses it either way.
 */
export function ActivityFiltersBar({
  filters,
  showActorFilters,
  refreshing,
  windowIssue,
  maxRangeDays,
  onChange,
  onReset,
  onRefresh,
}: {
  filters: ActivityFilters;
  /** True only when the server said this caller is in the global scope. */
  showActorFilters: boolean;
  refreshing: boolean;
  /** Why the named window cannot be queried, decided against THIS deployment's
   *  stated ceiling rather than a client constant. */
  windowIssue: WindowProblem | null;
  maxRangeDays: number;
  onChange: (next: ActivityFilters) => void;
  onReset: () => void;
  onRefresh: () => void;
}) {
  const t = useContent().operations;
  const patch = (next: Partial<ActivityFilters>) => onChange({ ...filters, ...next });
  const isDefault =
    JSON.stringify(filters) === JSON.stringify(DEFAULT_ACTIVITY_FILTERS);
  const days = String(maxRangeDays);

  return (
    <div
      role="group"
      aria-label={t.filtersAria}
      data-testid="activity-filters"
      className="flex-none flex items-end gap-2 flex-wrap"
    >
      <SelectFilter
        label={t.fRange}
        value={filters.preset}
        anyLabel={t.anyOption}
        width="w-[136px]"
        options={TIME_PRESETS.map((preset) => ({
          value: preset,
          label: t.rangePreset[preset],
        }))}
        onChange={(next) =>
          patch({
            preset: (next ?? '24h') as ActivityFilters['preset'],
            // Leaving a stale explicit window behind a preset would query a
            // range the control no longer shows.
            from: next === 'custom' ? filters.from : null,
            to: next === 'custom' ? filters.to : null,
          })
        }
      />
      {filters.preset === 'custom' && (
        <>
          <InstantFilter label={t.fFrom} value={filters.from} onChange={(from) => patch({ from })} />
          <InstantFilter label={t.fTo} value={filters.to} onChange={(to) => patch({ to })} />
        </>
      )}

      <SelectFilter
        label={t.fRecordKind}
        value={filters.recordKind}
        anyLabel={t.anyOption}
        width="w-[142px]"
        options={RECORD_KINDS.map((kind) => ({
          value: kind,
          label: t.recordKindFilter[kind],
        }))}
        onChange={(next) =>
          patch({ recordKind: (next ?? 'api_request') as ActivityFilters['recordKind'] })
        }
      />

      {showActorFilters && (
        <>
          <TextFilter
            label={t.fActorId}
            value={filters.actorId}
            inputMode="numeric"
            width="w-[110px]"
            parse={(raw) => parsePositiveInt(raw)}
            onCommit={(next) => patch({ actorId: next === null ? null : Number(next) })}
          />
          <TextFilter
            label={t.fActorLogin}
            value={filters.actorLogin}
            width="w-[140px]"
            parse={(raw) => parseLogin(raw)}
            onCommit={(next) => patch({ actorLogin: next === null ? null : String(next) })}
          />
        </>
      )}

      <SelectFilter
        label={t.fOperation}
        value={filters.operationId}
        anyLabel={t.anyOption}
        width="w-[196px]"
        groups={OPERATION_CATALOG.map((group) => ({
          label: t.operationGroup[group.group],
          options: group.ids.map((id) => ({ value: id, label: id })),
        }))}
        onChange={(operationId) => patch({ operationId })}
      />
      <SelectFilter
        label={t.fMethod}
        value={filters.method}
        anyLabel={t.anyOption}
        width="w-[102px]"
        options={METHODS.map((method) => ({ value: method, label: method }))}
        onChange={(method) => patch({ method })}
      />
      <SelectFilter
        label={t.fStatusClass}
        value={filters.statusClass}
        anyLabel={t.anyOption}
        width="w-[102px]"
        options={STATUS_CLASSES.map((cls) => ({ value: cls, label: cls }))}
        onChange={(statusClass) => patch({ statusClass })}
      />
      <TextFilter
        label={t.fStatusCode}
        value={filters.statusCode}
        inputMode="numeric"
        width="w-[92px]"
        parse={(raw) => parseStatusCode(raw)}
        onCommit={(next) => patch({ statusCode: next === null ? null : Number(next) })}
      />
      <SelectFilter
        label={t.fOutcome}
        value={filters.outcome}
        anyLabel={t.anyOption}
        width="w-[132px]"
        options={OUTCOMES.map((outcome) => ({ value: outcome, label: t.outcome[outcome] }))}
        onChange={(outcome) => patch({ outcome })}
      />
      <TextFilter
        label={t.fRepo}
        value={filters.repoFullName}
        width="w-[168px]"
        parse={(raw) => parseRepoFullName(raw)}
        onCommit={(next) => patch({ repoFullName: next === null ? null : String(next) })}
      />
      <TextFilter
        label={t.fTriggerIssue}
        value={filters.triggerIssue}
        inputMode="numeric"
        width="w-[100px]"
        parse={(raw) => parsePositiveInt(raw)}
        onCommit={(next) => patch({ triggerIssue: next === null ? null : Number(next) })}
      />
      <TextFilter
        label={t.fSessionId}
        value={filters.sessionId}
        width="w-[176px]"
        parse={(raw) => parseSessionId(raw)}
        onCommit={(next) => patch({ sessionId: next === null ? null : String(next) })}
      />
      <TextFilter
        label={t.fRequestId}
        value={filters.requestId}
        width="w-[176px]"
        parse={(raw) => parseRequestId(raw)}
        onCommit={(next) => patch({ requestId: next === null ? null : String(next) })}
      />

      <div className="flex items-center gap-2 flex-none pb-[1px]">
        <ResetFiltersButton label={t.resetFilters} disabled={isDefault} onClick={onReset} />
        <RefreshButton
          label={t.refresh}
          busyLabel={t.refreshing}
          busy={refreshing}
          onClick={onRefresh}
        />
      </div>

      {windowIssue !== null && (
        <p className="w-full font-mono text-[11px] text-warn" role="status">
          {t.rangeProblem[windowIssue].replace('{days}', days)}
        </p>
      )}
      {filters.preset === 'custom' && windowIssue === null && (
        <p className="w-full font-mono text-[10.5px] text-ghost">
          {t.rangeHint.replace('{days}', days)}
        </p>
      )}
    </div>
  );
}
