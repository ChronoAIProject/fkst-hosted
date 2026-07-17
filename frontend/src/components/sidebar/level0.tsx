import { useCallback, useState } from 'react';
import { useContent } from '@/i18n';
import { filterAccounts, packagesByAccount, sessionsByAccount } from '@/lib/api/derive';
import type { OverviewResponse } from '@/lib/api/types';
import { FIELD_INPUT } from '@/components/ui/field';
import { ConfirmDialog } from '@/components/modals/confirm-dialog';
import { CreateRepoModal } from '@/components/modals/create-repo-modal';
import type { UserRepo } from '@/components/modals/create-repo-modal';
import { AccountList } from './account-list';
import { CanvasBarChart, ChartScopeSelect } from './charts';
import { StatusLegend, ViewDescription } from './legend';

/** Level-0 sidebar: what the root view is, the legend, the account filter,
 *  the account list with connection actions, repo creation, and the two
 *  overview charts scoped by an account select. */
export function Level0Sidebar({
  overview,
  query,
  onQueryChange,
  onOpenAccount,
  onRepoCreated,
  onChanged,
}: {
  overview: OverviewResponse;
  query: string;
  onQueryChange: (q: string) => void;
  onOpenAccount: (login: string) => void;
  /** A repo was created via the modal — the page re-fetches and drills in. */
  onRepoCreated: (repo: UserRepo) => void;
  /** Installation state changed (uninstall) — the page re-fetches. */
  onChanged: () => void;
}) {
  const c = useContent().dashboard;
  const rc = c.repos;
  const cc = c.canvas;

  const [showCreate, setShowCreate] = useState(false);
  const [uninstallLogin, setUninstallLogin] = useState<string | null>(null);
  const [chartScope, setChartScope] = useState<string | null>(null);

  const accounts = overview.accounts;
  const shown = filterAccounts(accounts, query);
  const orgs = accounts.filter((a) => a.kind === 'org').map((a) => a.login);

  // Clamp the chart scope to the filtered options: a selection the name
  // filter removed falls back to "all", so the select never sits on an
  // invisible value while the charts silently scope to a hidden account.
  const scopeOptions = shown.map((a) => a.login);
  const effectiveScope =
    chartScope != null && scopeOptions.includes(chartScope) ? chartScope : null;

  const onUninstall = useCallback((login: string) => setUninstallLogin(login), []);
  const closeUninstall = useCallback(() => setUninstallLogin(null), []);
  const doneUninstall = useCallback(() => {
    setUninstallLogin(null);
    onChanged();
  }, [onChanged]);

  return (
    <div className="flex flex-col gap-4">
      <ViewDescription text={cc.viewRoot} />
      <StatusLegend />

      {overview.app_slug == null && (
        <p className="font-mono text-[12px] text-ghost">{rc.appNotConfigured}</p>
      )}

      <div className="flex items-center gap-2">
        <input
          type="search"
          value={query}
          onChange={(e) => onQueryChange(e.target.value)}
          placeholder={cc.filterAccountsPlaceholder}
          aria-label={cc.filterAccountsPlaceholder}
          className={FIELD_INPUT}
        />
        <button
          type="button"
          onClick={() => setShowCreate(true)}
          className="font-ui font-semibold text-[12px] bg-amber text-amber-ink rounded-control px-3 py-1.5 transition-colors hover:brightness-[1.06] cursor-pointer flex-none"
        >
          {rc.newRepo}
        </button>
      </div>

      {accounts.length === 0 ? (
        <p className="font-mono text-[12.5px] text-ghost">{cc.noAccounts}</p>
      ) : shown.length === 0 ? (
        <p className="font-mono text-[12.5px] text-ghost">{cc.noAccountsMatch}</p>
      ) : (
        <AccountList
          accounts={shown}
          appSlug={overview.app_slug}
          onOpenAccount={onOpenAccount}
          onUninstall={onUninstall}
        />
      )}

      <div className="border-t border-line pt-4 flex flex-col gap-4">
        <ChartScopeSelect
          id="chart-scope-account"
          label={cc.chartScopeAriaAccounts}
          allLabel={cc.chartScopeAllAccounts}
          options={scopeOptions}
          value={effectiveScope}
          onChange={setChartScope}
        />
        <CanvasBarChart
          title={cc.chartSessionsTitle}
          rows={sessionsByAccount(shown, effectiveScope)}
          hue="amber"
        />
        <CanvasBarChart
          title={cc.chartPackagesTitle}
          rows={packagesByAccount(shown, effectiveScope)}
          hue="green"
        />
      </div>

      {showCreate && (
        <CreateRepoModal
          viewerLogin={overview.viewer.login}
          orgs={orgs}
          rc={rc}
          onClose={() => setShowCreate(false)}
          onCreated={(repo) => {
            setShowCreate(false);
            onRepoCreated(repo);
          }}
        />
      )}

      {uninstallLogin != null && (
        <ConfirmDialog
          title={rc.uninstallConfirmTitle.replace('{owner}', uninstallLogin)}
          body={rc.uninstallConfirmBody.replace('{owner}', uninstallLogin)}
          confirmLabel={rc.uninstallConfirm}
          pendingLabel={rc.uninstallPending}
          cancelLabel={rc.cancel}
          path={`/api/v1/installations/${encodeURIComponent(uninstallLogin)}`}
          fallbackError={rc.uninstallFailed}
          onClose={closeUninstall}
          onDone={doneUninstall}
        />
      )}
    </div>
  );
}
