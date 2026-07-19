import { cn } from '@/lib/utils';
import { useContent } from '@/i18n';
import type { AccountOverview, RepoOverview } from '@/lib/api/types';
import { Chip } from '@/components/ui/chip';
import { StaggerItem } from '@/components/ui/motion';

/** One repository row of the level-1 sidebar: GitHub link, visibility/org
 *  chips, and the App install affordance (the old dashboard's RepoRow
 *  pattern — Install link with the non-admin hint, or the installed mark
 *  with the manage-on-GitHub hint). */
function RepoRow({
  repo,
  isOrg,
  appSlug,
  installedViaInstallation,
  highlight,
  onOpenRepo,
}: {
  repo: RepoOverview;
  isOrg: boolean;
  appSlug: string | null;
  /** True when the account has an installation — adding/removing this repo is
   * done on that installation's GitHub settings page (the account's Manage
   * link), since GitHub allows per-repo selection changes only there. Drives
   * the explanatory tooltip on the installed mark. */
  installedViaInstallation: boolean;
  highlight: boolean;
  onOpenRepo: (owner: string, name: string) => void;
}) {
  const c = useContent().dashboard;
  const rc = c.repos;
  const cc = c.canvas;
  const full = `${repo.owner}/${repo.name}`;

  return (
    <div
      className={cn(
        'flex items-center gap-2 py-2 px-2 -mx-2 rounded-control text-[12.5px] min-w-0',
        'transition-[background-color,box-shadow] hover:bg-[color-mix(in_oklab,var(--raise-2)_80%,transparent)] hover:shadow-1',
        highlight && 'anim-repo-pulse'
      )}
    >
      <a
        href={`https://github.com/${full}`}
        target="_blank"
        rel="noreferrer"
        className="hover-underline font-mono text-[12px] text-fg hover:text-amber transition-colors truncate min-w-0"
      >
        {full}
      </a>
      <Chip tone="neutral">{repo.private ? rc.private : rc.public}</Chip>
      {isOrg && <Chip tone="amber">{rc.org}</Chip>}
      <button
        type="button"
        onClick={() => onOpenRepo(repo.owner, repo.name)}
        aria-label={cc.openRepoAria.replace('{repo}', full)}
        className="font-mono text-[12px] text-dim hover:text-fg transition-colors cursor-pointer px-1 rounded-chip flex-none"
      >
        →
      </button>
      <span className="flex-1" aria-hidden="true" />
      {repo.active_sessions > 0 && (
        <span className="font-mono text-[10.5px] text-amber flex-none">
          {cc.statusActiveCount.replace('{n}', String(repo.active_sessions))}
        </span>
      )}
      {/* Persistent status derived purely from repo state: a not-yet-installed
          repo carries the badge on every visit until the App is actually
          installed — unlike the transient freshly-created callout below, which
          only appears the one time you drill in right after creating it. */}
      {!repo.installed && appSlug != null && (
        <span className="flex-none" title={cc.needsInstallHint}>
          <Chip tone="amber">{cc.needsInstall}</Chip>
        </span>
      )}
      {repo.installed ? (
        <span
          className="font-mono text-[11px] text-green flex-none"
          title={installedViaInstallation ? rc.manageRepoHint : undefined}
        >{`✓ ${rc.installed}`}</span>
      ) : (
        appSlug != null && (
          <a
            href={`https://github.com/apps/${appSlug}/installations/new`}
            target="_blank"
            rel="noreferrer"
            title={repo.admin ? undefined : rc.nonAdminHint}
            className="font-ui font-semibold text-[11.5px] bg-grad-accent text-amber-ink rounded-control px-3 py-1 shadow-[var(--shadow-1),var(--glow-amber)] transition-[filter,box-shadow] hover:brightness-110 flex-none"
          >
            {rc.install}
          </a>
        )
      )}
    </div>
  );
}

/** Level-1 sidebar repo list with the freshly-created-repo callout. */
export function RepoList({
  account,
  repos,
  appSlug,
  createdKey,
  onOpenRepo,
}: {
  account: AccountOverview;
  /** Already name-filtered repos of the account. */
  repos: RepoOverview[];
  appSlug: string | null;
  /** `owner/name` of a repo created via the modal — highlighted + guided. */
  createdKey: string | null;
  onOpenRepo: (owner: string, name: string) => void;
}) {
  const rc = useContent().dashboard.repos;

  return (
    <div className="flex flex-col divide-y divide-[color-mix(in_oklab,var(--line)_55%,transparent)]">
      {repos.map((repo, i) => {
        const key = `${repo.owner}/${repo.name}`;
        const isNew = key === createdKey;
        return (
          // Staggered entrance per row (reduced-motion-safe via .anim-row-in).
          <StaggerItem key={key} index={i}>
            <RepoRow
              repo={repo}
              isOrg={account.kind === 'org'}
              appSlug={appSlug}
              installedViaInstallation={account.installation_id != null}
              highlight={isNew}
              onOpenRepo={onOpenRepo}
            />
            {isNew && !repo.installed && appSlug != null && (
              <div className="anim-notice-in grad-border grad-border-accent mb-2 flex items-center gap-3 flex-wrap rounded-card px-3 py-2 text-[12.5px] text-dim shadow-[var(--shadow-2),var(--glow-amber)]">
                <span>{rc.createdNextStep}</span>
                <a
                  href={`https://github.com/apps/${appSlug}/installations/new`}
                  target="_blank"
                  rel="noreferrer"
                  className="anim-sheen font-ui font-semibold text-[11.5px] bg-grad-accent text-amber-ink rounded-control px-3 py-1 shadow-[var(--shadow-1),var(--glow-amber)] transition-[filter,box-shadow] hover:brightness-110 flex-none"
                >
                  {rc.install}
                </a>
              </div>
            )}
          </StaggerItem>
        );
      })}
    </div>
  );
}
