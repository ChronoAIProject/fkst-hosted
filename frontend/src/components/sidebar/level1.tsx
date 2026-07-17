import { useState } from 'react';
import { useContent } from '@/i18n';
import { filterRepos, packagesByRepo, sessionsByRepo } from '@/lib/api/derive';
import type { AccountOverview } from '@/lib/api/types';
import { FIELD_INPUT } from '@/components/ui/field';
import { CanvasBarChart, ChartScopeSelect } from './charts';
import { StatusLegend, ViewDescription } from './legend';
import { RepoList } from './repo-list';

/** Level-1 sidebar: what this account view is, the legend, the repo filter,
 *  the repo list with the App install affordance, and the two charts scoped
 *  to the account (further scopable to one repo). */
export function Level1Sidebar({
  account,
  appSlug,
  query,
  onQueryChange,
  createdKey,
  onOpenRepo,
}: {
  account: AccountOverview;
  appSlug: string | null;
  query: string;
  onQueryChange: (q: string) => void;
  /** `owner/name` of a freshly created repo to highlight, if any. */
  createdKey: string | null;
  onOpenRepo: (owner: string, name: string) => void;
}) {
  const c = useContent().dashboard;
  const rc = c.repos;
  const cc = c.canvas;
  const [chartScope, setChartScope] = useState<string | null>(null);

  const shown = filterRepos(account.repos, query);

  // Clamp the chart scope to the filtered options (same rule as level 0): a
  // selection the name filter removed falls back to "all repositories".
  const scopeOptions = shown.map((r) => r.name);
  const effectiveScope =
    chartScope != null && scopeOptions.includes(chartScope) ? chartScope : null;

  return (
    <div className="flex flex-col gap-4">
      <ViewDescription text={cc.viewAccount.replace('{login}', account.login)} />
      <StatusLegend />

      {appSlug == null && <p className="font-mono text-[12px] text-ghost">{rc.appNotConfigured}</p>}

      <input
        type="search"
        value={query}
        onChange={(e) => onQueryChange(e.target.value)}
        placeholder={cc.filterReposPlaceholder}
        aria-label={cc.filterReposPlaceholder}
        className={FIELD_INPUT}
      />

      {account.repos.length === 0 ? (
        <p className="font-mono text-[12px] text-ghost italic">{rc.groupEmpty}</p>
      ) : shown.length === 0 ? (
        <p className="font-mono text-[12.5px] text-ghost">{cc.noReposMatch}</p>
      ) : (
        <RepoList
          account={account}
          repos={shown}
          appSlug={appSlug}
          createdKey={createdKey}
          onOpenRepo={onOpenRepo}
        />
      )}

      <div className="border-t border-line pt-4 flex flex-col gap-4">
        <ChartScopeSelect
          id="chart-scope-repo"
          label={cc.chartScopeAriaRepos}
          allLabel={cc.chartScopeAllRepos}
          options={scopeOptions}
          value={effectiveScope}
          onChange={setChartScope}
        />
        {/* Both charts consume the same filtered set, so they always describe
            one population — the repos listed above them. */}
        <CanvasBarChart
          title={cc.chartSessionsTitle}
          rows={sessionsByRepo(shown, effectiveScope)}
          hue="amber"
        />
        <CanvasBarChart
          title={cc.chartPackagesTitle}
          rows={packagesByRepo(shown, effectiveScope)}
          hue="green"
        />
      </div>
    </div>
  );
}
