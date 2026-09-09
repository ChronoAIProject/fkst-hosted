import { useContent } from '@/i18n';
import { ATTRIBUTION_SOURCES, SANDBOX_BACKENDS, SANDBOX_STATUSES } from '@/lib/api/operations';
import type { SandboxFilters } from '@/lib/operations/state';
import {
  DEFAULT_SANDBOX_FILTERS,
  parseLogin,
  parsePositiveInt,
  parseRepoFullName,
  parseSessionId,
} from '@/lib/operations/state';
import { ResetFiltersButton, SelectFilter, TextFilter } from './filter-controls';
import { RefreshButton } from './parts';

/**
 * The Sandboxes view's filter toolbar.
 *
 * Every control here NARROWS an already-authorized set. The creator and
 * repository filters look like access controls and are not: the server applied
 * the access registry before it applied any of these, so filtering by another
 * person's login can only ever remove rows the caller could already see. The
 * note under the toolbar says exactly that, because a reader who believes
 * otherwise will misread an empty result as a permission grant.
 */
export function SandboxFiltersBar({
  filters,
  refreshing,
  onChange,
  onReset,
  onRefresh,
}: {
  filters: SandboxFilters;
  refreshing: boolean;
  onChange: (next: SandboxFilters) => void;
  onReset: () => void;
  onRefresh: () => void;
}) {
  const t = useContent().operations;
  const patch = (next: Partial<SandboxFilters>) => onChange({ ...filters, ...next });
  const isDefault = JSON.stringify(filters) === JSON.stringify(DEFAULT_SANDBOX_FILTERS);

  return (
    <div
      role="group"
      aria-label={t.filtersAria}
      data-testid="sandbox-filters"
      className="flex-none flex items-end gap-2 flex-wrap"
    >
      <SelectFilter
        label={t.fStatus}
        value={filters.status}
        anyLabel={t.anyOption}
        width="w-[142px]"
        options={SANDBOX_STATUSES.map((status) => ({
          value: status,
          label: t.sandboxStatus[status],
        }))}
        onChange={(status) => patch({ status })}
      />
      <SelectFilter
        label={t.fBackend}
        value={filters.backend}
        anyLabel={t.anyOption}
        width="w-[142px]"
        options={SANDBOX_BACKENDS.map((backend) => ({
          value: backend,
          label: t.backendKind[backend],
        }))}
        onChange={(backend) => patch({ backend })}
      />
      <TextFilter
        label={t.fCreatorId}
        value={filters.creatorId}
        inputMode="numeric"
        width="w-[110px]"
        parse={(raw) => parsePositiveInt(raw)}
        onCommit={(next) => patch({ creatorId: next === null ? null : Number(next) })}
      />
      <TextFilter
        label={t.fCreatorLogin}
        value={filters.creatorLogin}
        width="w-[148px]"
        parse={(raw) => parseLogin(raw)}
        onCommit={(next) => patch({ creatorLogin: next === null ? null : String(next) })}
      />
      <TextFilter
        label={t.fRepo}
        value={filters.repoFullName}
        width="w-[176px]"
        parse={(raw) => parseRepoFullName(raw)}
        onCommit={(next) => patch({ repoFullName: next === null ? null : String(next) })}
      />
      <TextFilter
        label={t.fTriggerIssue}
        value={filters.triggerIssue}
        inputMode="numeric"
        width="w-[104px]"
        parse={(raw) => parsePositiveInt(raw)}
        onCommit={(next) => patch({ triggerIssue: next === null ? null : Number(next) })}
      />
      <TextFilter
        label={t.fSessionId}
        value={filters.sessionId}
        width="w-[184px]"
        parse={(raw) => parseSessionId(raw)}
        onCommit={(next) => patch({ sessionId: next === null ? null : String(next) })}
      />
      <SelectFilter
        label={t.fAttribution}
        value={filters.attributionSource}
        anyLabel={t.anyOption}
        width="w-[184px]"
        options={ATTRIBUTION_SOURCES.map((source) => ({
          value: source,
          label: t.attribution[source],
        }))}
        onChange={(attributionSource) => patch({ attributionSource })}
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

      <p className="w-full font-mono text-[10.5px] text-ghost">{t.filterScopeNote}</p>
    </div>
  );
}
