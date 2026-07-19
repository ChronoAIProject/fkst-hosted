import { useContent } from '@/i18n';
import type { AccountOverview } from '@/lib/api/types';
import { StaggerItem } from '@/components/ui/motion';

/** Exact GitHub settings page for an installation on this account. */
export function manageUrl(login: string, personal: boolean, installationId: number): string {
  return personal
    ? `https://github.com/settings/installations/${installationId}`
    : `https://github.com/organizations/${login}/settings/installations/${installationId}`;
}

/** Level-0 sidebar list: one row per account with per-account install counts
 *  and the App connection actions (Connect / Manage / Uninstall). */
export function AccountList({
  accounts,
  appSlug,
  onOpenAccount,
  onUninstall,
}: {
  accounts: AccountOverview[];
  appSlug: string | null;
  onOpenAccount: (login: string) => void;
  onUninstall: (login: string) => void;
}) {
  const c = useContent().dashboard;
  const rc = c.repos;
  const cc = c.canvas;

  return (
    <div className="flex flex-col gap-4">
      {accounts.map((account, i) => {
        const installedCount = account.repos.filter((r) => r.installed).length;
        return (
          // Staggered entrance for the account rows; collapses to the final
          // state under prefers-reduced-motion (the .anim-row-in class is
          // disabled there in index.css).
          <StaggerItem key={account.login} index={i} className="flex flex-col gap-1">
            <div className="flex items-center gap-2.5 flex-wrap">
              <span className="font-mono text-eyebrow text-ghost uppercase">
                {account.kind === 'personal' ? rc.personalGroup : rc.orgGroup}
              </span>
              <h3 className="font-display font-semibold text-[14.5px] text-fg">{account.login}</h3>
              <button
                type="button"
                onClick={() => onOpenAccount(account.login)}
                aria-label={cc.openAccountAria.replace('{login}', account.login)}
                className="font-mono text-[12px] text-dim hover:text-fg transition-colors cursor-pointer px-1 rounded-chip"
              >
                →
              </button>
              <span className="flex-1" aria-hidden="true" />
              <span className="font-mono text-[11px] text-ghost">
                {rc.groupCounts
                  .replace('{installed}', String(installedCount))
                  .replace('{total}', String(account.repos.length))}
              </span>
              {account.installation_id != null ? (
                <span className="flex items-center gap-1.5">
                  <a
                    href={manageUrl(
                      account.login,
                      account.kind === 'personal',
                      account.installation_id
                    )}
                    target="_blank"
                    rel="noreferrer"
                    className="font-ui font-semibold text-[11px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg transition-colors"
                  >
                    {rc.manage}
                  </a>
                  <button
                    type="button"
                    onClick={() => onUninstall(account.login)}
                    className="font-ui font-semibold text-[11px] border border-line rounded-control px-2.5 py-1 text-red transition-colors hover:border-[color-mix(in_oklab,var(--red)_45%,var(--line))] cursor-pointer"
                  >
                    {rc.uninstall}
                  </button>
                </span>
              ) : (
                appSlug != null && (
                  <span className="flex items-center gap-2">
                    <span className="font-ui text-[11px] text-ghost">{rc.connectHint}</span>
                    <a
                      href={`https://github.com/apps/${appSlug}/installations/new`}
                      target="_blank"
                      rel="noreferrer"
                      className="font-ui font-semibold text-[11.5px] bg-amber text-amber-ink rounded-control px-3 py-1 transition-colors hover:brightness-[1.06]"
                    >
                      {rc.connect}
                    </a>
                  </span>
                )
              )}
            </div>
            {account.repos.length === 0 && (
              <p className="font-mono text-[12px] text-ghost italic py-1">{rc.groupEmpty}</p>
            )}
          </StaggerItem>
        );
      })}
    </div>
  );
}
