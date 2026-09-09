import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { Chip } from '@/components/ui/chip';
import { ErrorNote } from '@/components/ui/error-note';
import { ConfirmDialog } from '@/components/modals/confirm-dialog';
import { useContent } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { RESTORED_UNKNOWN, useChat } from './chat-context';
import type { ChatProposal } from './chat-context';
import { CardBody } from './action-card-bodies';
import { runProposalAsMutation } from './proposal-exec';
import {
  destructiveConfirm,
  kindLabel,
  outcomeChip,
  requiredSecretKeys,
  scopeLine,
} from './proposal-meta';

/**
 * A confirm-gated proposal card.
 *
 * The human click IS the authorization boundary: the chat layer holds no write
 * capability, and confirming calls the same typed API function the dashboard's own
 * buttons use, under the user's own token. Nothing here ever auto-executes —
 * including on a transcript restore.
 *
 * Three kinds have no undo (retire a session, delete an environment, uninstall the App)
 * and route through `ConfirmDialog` instead of executing on the first click.
 */
export function ActionCard({ entry }: { entry: ChatProposal }) {
  const s = useContent().chat;
  const { apiFetch } = useAuth();
  const { executeProposal, dismissProposal, markProposalSucceeded } = useChat();
  const { proposal, state } = entry;
  const [confirmOpen, setConfirmOpen] = useState(false);
  /** Secret VALUES the user typed. Component state only: never lifted into the
   *  transcript, never persisted, and gone the moment the card unmounts. */
  const [secrets, setSecrets] = useState<Record<string, string>>({});

  const executing = state === 'executing';
  const succeeded = state === 'succeeded';
  // A restored mid-flight proposal is `failed` with a sentinel, because its real
  // outcome is unknowable — the copy says exactly that rather than pretending.
  const errorText = entry.error === RESTORED_UNKNOWN ? s.restoredUnknown : (entry.error ?? null);

  const secretKeys = requiredSecretKeys(proposal);
  const missingSecret = secretKeys.some((key) => (secrets[key] ?? '').trim() === '');
  const danger = useMemo(() => destructiveConfirm(proposal, s), [proposal, s]);

  if (succeeded) {
    const dashboardHref =
      proposal.kind === 'create_session' && entry.issueNumber != null
        ? `/dashboard?owner=${encodeURIComponent(proposal.owner)}&repo=${encodeURIComponent(
            proposal.name
          )}&session=trigger-${entry.issueNumber}`
        : null;
    return (
      <div
        data-testid="chat-action-card"
        data-state="succeeded"
        className="rounded-card border border-line bg-raise px-2.5 py-2 flex items-center gap-2 flex-wrap"
      >
        <Chip tone="green">{outcomeChip(proposal, s)}</Chip>
        <span className="font-mono text-[10.5px] text-faint">{scopeLine(proposal, s)}</span>
        <span className="flex-1" aria-hidden="true" />
        {entry.issueUrl && (
          <a
            href={entry.issueUrl}
            target="_blank"
            rel="noopener noreferrer"
            data-testid="chat-action-issue-link"
            className="rounded-chip border border-line-2 bg-raise-2 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.1em] text-faint no-underline transition-colors hover:text-fg"
          >
            {s.openIssue} ↗
          </a>
        )}
        {proposal.kind === 'create_repository' && (
          <a
            href={`https://github.com/${proposal.owner ?? ''}${proposal.owner ? '/' : ''}${proposal.name}`}
            target="_blank"
            rel="noopener noreferrer"
            data-testid="chat-action-repo-link"
            className="rounded-chip border border-line-2 bg-raise-2 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.1em] text-faint no-underline transition-colors hover:text-fg"
          >
            {s.openRepo} ↗
          </a>
        )}
        {dashboardHref && (
          <Link
            to={dashboardHref}
            data-testid="chat-action-dashboard-link"
            className="rounded-chip border border-line-2 bg-raise-2 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.1em] text-faint no-underline transition-colors hover:text-fg"
          >
            {s.openInDashboard}
          </Link>
        )}
      </div>
    );
  }

  return (
    <div
      data-testid="chat-action-card"
      data-state={state}
      data-kind={proposal.kind}
      className="rounded-card border border-line bg-glass backdrop-blur-glass p-2.5 flex flex-col gap-2 shadow-[var(--shadow-1),var(--highlight-top)]"
    >
      <div className="flex items-center gap-2 flex-wrap">
        <Chip tone="neutral">{kindLabel(proposal, s)}</Chip>
        <span className="font-mono text-[10.5px] text-ghost">{scopeLine(proposal, s)}</span>
      </div>
      <p className="text-[12.5px] leading-relaxed text-dim">{proposal.summary}</p>

      <CardBody
        proposal={proposal}
        secrets={secrets}
        onSecretChange={(key, value) => setSecrets((current) => ({ ...current, [key]: value }))}
      />

      <p className="font-mono text-[10px] text-ghost">
        {proposal.target.method} {proposal.target.path}
      </p>
      {/* Said plainly, because the proposal validator deliberately does NOT check
          repository authority or label collisions — those run at confirmation. */}
      <p className="font-mono text-[10px] text-ghost">{s.finalChecksNote}</p>

      {/* A destructive kind routes through ConfirmDialog, which owns its own inline
          error — so this card shows none for those (its verified contract). */}
      {errorText != null && danger == null && <ErrorNote message={errorText} />}
      {missingSecret && (
        <p data-testid="chat-env-secrets-required" className="font-mono text-[10px] text-warn">
          {s.envSecretsRequired}
        </p>
      )}

      <div className="flex items-center gap-2">
        <button
          type="button"
          disabled={executing || missingSecret}
          onClick={() => {
            if (danger != null) {
              setConfirmOpen(true);
              return;
            }
            void executeProposal(entry.id, { secrets });
          }}
          data-testid="chat-action-confirm"
          className="rounded-control bg-grad-accent px-3 py-1.5 font-ui text-[12px] font-semibold text-amber-ink shadow-[var(--shadow-1),var(--glow-amber)] transition-[filter,opacity] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-50 cursor-pointer"
        >
          {executing ? (
            <>
              {s.executing}
              <span aria-hidden="true" className="anim-dot-blink ml-1 inline-block">
                ·
              </span>
            </>
          ) : (
            s.confirmExecute
          )}
        </button>
        <button
          type="button"
          disabled={executing}
          onClick={() => dismissProposal(entry.id)}
          data-testid="chat-action-dismiss"
          className="glass grad-border rounded-control px-3 py-1.5 font-ui text-[12px] font-semibold text-dim transition-colors hover:bg-raise-2 disabled:cursor-not-allowed disabled:opacity-50 cursor-pointer"
        >
          {s.dismiss}
        </button>
      </div>

      {confirmOpen && danger != null && (
        <ConfirmDialog
          title={danger.title}
          body={danger.body}
          confirmLabel={danger.action}
          pendingLabel={s.executing}
          cancelLabel={s.dismiss}
          // The dialog runs the mutation and owns its own error/close. The card's
          // own state is then updated through markProposalSucceeded on success, so
          // the success row renders exactly as it does for the other kinds.
          action={() => runProposalAsMutation(apiFetch, proposal, { secrets })}
          fallbackError={s.executeFailed}
          onClose={() => setConfirmOpen(false)}
          onDone={() => {
            setConfirmOpen(false);
            // The dialog ALREADY ran the mutation (its verified contract), so this
            // only records the outcome — calling executeProposal here would run the
            // destructive action a second time.
            markProposalSucceeded(entry.id);
          }}
        />
      )}
    </div>
  );
}

/** The cards under one assistant message. */
export function ActionCards({ proposals }: { proposals: ChatProposal[] }) {
  if (proposals.length === 0) return null;
  return (
    <div data-testid="chat-action-cards" className="flex flex-col gap-1.5">
      {proposals.map((entry) => (
        <ActionCard key={entry.id} entry={entry} />
      ))}
    </div>
  );
}
