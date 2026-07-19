import { cn } from '@/lib/utils';
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
    <div className="flex flex-col gap-3">
      {accounts.map((account, i) => {
        const installedCount = account.repos.filter((r) => r.installed).length;
        const hasActive = account.repos.some((r) => r.active_sessions > 0);
        // Resting depth + a status-matched glow so a connected/active account
        // reads at a glance: amber bloom when sessions are running, plain card
        // depth when connected, the quietest depth when not yet installed.
        const statusGlow = hasActive
          ? 'shadow-[var(--shadow-2),var(--glow-amber)]'
          : account.installation_id != null
            ? 'shadow-2'
            : 'shadow-1';
        return (
          // Staggered entrance for the account rows (the .anim-row-in transform
          // lives on the StaggerItem, disabled under prefers-reduced-motion);
          // the inner card owns the hover-lift so the two transforms never fight.
          <StaggerItem key={account.login} index={i}>
            <div
              className={cn(
                'grad-border hover-lift rounded-card px-3.5 py-3 flex flex-col gap-1',
                statusGlow
              )}
            >
              <div className="flex items-center gap-2.5 flex-wrap">
                <span className="font-mono text-eyebrow text-ghost uppercase">
                  {account.kind === 'personal' ? rc.personalGroup : rc.orgGroup}
                </span>
                <h3 className="grad-text grad-text-fg font-display font-semibold text-[14.5px]">
                  {account.login}
                </h3>
                <button
                  type="button"
                  onClick={() => onOpenAccount(account.login)}
                  aria-label={cc.openAccountAria.replace('{login}', account.login)}
                  className="font-mono text-[12px] text-dim hover:text-amber transition-colors cursor-pointer px-1 rounded-chip"
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
                      className="font-ui font-semibold text-[11px] border border-line rounded-control px-2.5 py-1 text-dim transition-[color,border-color,box-shadow] hover:text-fg hover:border-line-2 hover:shadow-glow-amber"
                    >
                      {rc.manage}
                    </a>
                    <button
                      type="button"
                      onClick={() => onUninstall(account.login)}
                      className="font-ui font-semibold text-[11px] border border-line rounded-control px-2.5 py-1 text-red transition-[color,border-color,box-shadow] hover:border-[color-mix(in_oklab,var(--red)_45%,var(--line))] hover:shadow-glow-red cursor-pointer"
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
                        className="anim-sheen font-ui font-semibold text-[11.5px] bg-grad-accent text-amber-ink rounded-control px-3 py-1 shadow-[var(--shadow-1),var(--glow-amber)] transition-[filter,box-shadow] hover:brightness-110"
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
            </div>
          </StaggerItem>
        );
      })}
    </div>
  );
}
