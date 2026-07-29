import { useState } from 'react';
import { Chip } from '@/components/ui/chip';
import { MarkdownPreview } from '@/components/ui/markdown-preview';
import { Reveal } from '@/components/ui/motion';
import { useContent } from '@/i18n';
import type { ActionProposal } from './action-types';

/**
 * The per-kind card bodies: what will actually be created, changed, or removed.
 *
 * The confirm gate is only meaningful if the user can SEE what they are approving, so
 * each body shows the real payload rather than a description of it — the exact issue
 * body, the exact install commands, the exact account.
 *
 * Split out of `action-card.tsx` because the shell (chip, buttons, confirm modal,
 * success row) and the payload rendering change for different reasons: a new proposal
 * kind adds a body here and nothing there.
 */

type ChatCopy = ReturnType<typeof useContent>['chat'];

/** A label/value row in the small mono field table the cards share. */
function Fields({ rows }: { rows: [string, string][] }) {
  return (
    <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 font-mono text-[10.5px]">
      {rows.map(([label, value]) => (
        <div key={label} className="contents">
          <dt className="text-ghost uppercase tracking-[0.1em]">{label}</dt>
          <dd className="text-faint break-words">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

/** A destructive body: warn-toned rule plus the reason the assistant gave.
 *  Styled with --warn, never the brand accent — the accent means "fkst", not "careful". */
function DangerBody({ line, reason }: { line: string; reason?: string }) {
  return (
    <div className="flex flex-col gap-1 border-l-2 border-l-warn pl-2.5">
      <p className="font-mono text-[11.5px] text-warn">{line}</p>
      {reason != null && reason !== '' && (
        <p className="text-[12px] leading-relaxed text-dim">{reason}</p>
      )}
    </div>
  );
}

function SessionBody({
  proposal,
  s,
}: {
  proposal: Extract<ActionProposal, { kind: 'create_session' }>;
  s: ChatCopy;
}) {
  const [previewOpen, setPreviewOpen] = useState(false);
  const { request } = proposal;
  return (
    <div className="flex flex-col gap-2">
      {/* Collapsed by default: the exact issue body is the authoritative preview,
          but it is long, and the field table answers most questions at a glance. */}
      <button
        type="button"
        onClick={() => setPreviewOpen((open) => !open)}
        aria-expanded={previewOpen}
        data-testid="chat-action-preview-toggle"
        className="self-start rounded-control font-mono text-[10px] uppercase tracking-[0.12em] text-ghost transition-colors hover:text-faint cursor-pointer"
      >
        {s.previewToggle} {previewOpen ? '▴' : '▾'}
      </button>
      <Reveal open={previewOpen}>
        <pre
          data-testid="chat-action-preview"
          className="max-h-64 overflow-auto rounded-control border border-line bg-glass px-3 py-2 font-mono text-[11.5px] leading-5 text-dim whitespace-pre-wrap"
        >
          {proposal.rendered_issue_body}
        </pre>
      </Reveal>
      <Fields
        rows={[
          [s.fieldWorkLabel, request.work_label ?? s.fieldAutoDiscovered],
          [s.fieldPackages, String(request.packages.length + request.manifests.length)],
          [
            s.fieldBranches,
            `${request.source_branch ?? s.fieldDefault} → ${request.target_branch ?? 'fkst-hosted-default'}`,
          ],
          [s.fieldAutoMerge, request.auto_merge ? s.on : s.off],
          ...(request.environment
            ? ([[s.fieldEnvironment, request.environment]] as [string, string][])
            : []),
        ]}
      />
    </div>
  );
}

/** The environment card: install commands, variables, and a masked input per declared
 *  secret NAME.
 *
 *  The inputs are the whole reason a secret never has to reach the model: it drafts the
 *  names, the user supplies the values here, and they go straight to the API call on
 *  confirm without passing through the transcript or `sessionStorage`. */
function EnvironmentBody({
  proposal,
  s,
  secrets,
  onSecretChange,
}: {
  proposal: Extract<ActionProposal, { kind: 'save_environment_profile' }>;
  s: ChatCopy;
  secrets: Record<string, string>;
  onSecretChange: (key: string, value: string) => void;
}) {
  const replaceNote =
    proposal.replaces_existing === true
      ? s.envReplaceNote
      : proposal.replaces_existing === false
        ? s.envCreateNote
        : s.envUnknownNote;

  return (
    <div className="flex flex-col gap-2">
      <p
        data-testid="chat-env-replace-note"
        className={`font-mono text-[10.5px] ${
          proposal.replaces_existing === false ? 'text-ghost' : 'text-warn'
        }`}
      >
        {replaceNote}
      </p>

      <div className="flex flex-col gap-1">
        <span className="font-mono text-[10px] uppercase tracking-[0.1em] text-ghost">
          {s.envInstall}
        </span>
        <pre
          data-testid="chat-env-install"
          className="max-h-40 overflow-auto rounded-control border border-line bg-glass px-2.5 py-1.5 font-mono text-[11px] leading-5 text-dim whitespace-pre-wrap"
        >
          {proposal.install.map((command) => `$ ${command}`).join('\n')}
        </pre>
      </div>

      <Fields
        rows={[
          [
            s.envVariables,
            proposal.variables.length === 0
              ? s.envNoVariables
              : proposal.variables.map((entry) => `${entry.key}=${entry.value}`).join('  '),
          ],
        ]}
      />

      {proposal.secret_keys.length > 0 && (
        <div data-testid="chat-env-secrets" className="flex flex-col gap-1.5">
          <span className="font-mono text-[10px] uppercase tracking-[0.1em] text-ghost">
            {s.envSecrets}
          </span>
          {proposal.secret_keys.map((key) => (
            <label key={key} className="flex items-center gap-2">
              <span className="w-32 shrink-0 truncate font-mono text-[10.5px] text-faint">
                {key}
              </span>
              <input
                type="password"
                // Browser autofill on a credential field the user is TYPING for a
                // one-shot API call would offer the wrong password entirely.
                autoComplete="off"
                spellCheck={false}
                value={secrets[key] ?? ''}
                onChange={(event) => onSecretChange(key, event.target.value)}
                placeholder={s.envSecretPlaceholder}
                aria-label={key}
                data-testid={`chat-env-secret-${key}`}
                className="min-w-0 flex-1 rounded-control border border-line-2 bg-raise-2 px-2 py-1 font-mono text-[11px] text-fg outline-none transition-colors placeholder:text-ghost focus:border-[color-mix(in_oklab,var(--amber)_45%,var(--line-2))]"
              />
            </label>
          ))}
          <p className="font-mono text-[10px] leading-4 text-ghost">{s.envSecretHint}</p>
        </div>
      )}

      <p className="font-mono text-[10px] leading-4 text-ghost">{s.envValidateNote}</p>
    </div>
  );
}

/** The body for one proposal. */
export function CardBody({
  proposal,
  secrets,
  onSecretChange,
}: {
  proposal: ActionProposal;
  secrets: Record<string, string>;
  onSecretChange: (key: string, value: string) => void;
}) {
  const s = useContent().chat;

  switch (proposal.kind) {
    case 'create_session':
      return <SessionBody proposal={proposal} s={s} />;

    case 'create_work_item':
      return (
        <div className="flex flex-col gap-2">
          <p className="font-ui text-[12.5px] font-semibold text-fg">{proposal.title}</p>
          {proposal.label && (
            <span className="font-mono text-[10.5px] text-faint">{proposal.label}</span>
          )}
          {proposal.body !== '' && (
            <MarkdownPreview markdown={proposal.body} ariaLabel={s.workItemBodyAria} />
          )}
        </div>
      );

    case 'stop_session':
      return (
        <DangerBody
          line={s.stopTriggerLine.replace('{number}', String(proposal.trigger_issue_number))}
          reason={proposal.reason}
        />
      );

    case 'create_repository':
      return (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="font-ui text-[12.5px] font-semibold text-fg">{proposal.name}</span>
            {/* Visibility is the one irreversible-ish choice here, so it gets a chip
                rather than a table row: red for public, because publishing code the
                user meant to keep private is the mistake worth flagging. */}
            <Chip tone={proposal.private ? 'neutral' : 'red'}>
              {proposal.private ? s.repoPrivate : s.repoPublic}
            </Chip>
          </div>
          {proposal.description != null && proposal.description !== '' && (
            <p className="text-[12px] leading-relaxed text-dim">{proposal.description}</p>
          )}
          <p className="font-mono text-[10px] leading-4 text-ghost">{s.repoInstallNote}</p>
        </div>
      );

    case 'save_environment_profile':
      return (
        <EnvironmentBody
          proposal={proposal}
          s={s}
          secrets={secrets}
          onSecretChange={onSecretChange}
        />
      );

    case 'delete_environment_profile':
      return <DangerBody line={s.deleteEnvLine.replace('{name}', proposal.profile_name)} />;

    case 'uninstall_app':
      return (
        <DangerBody
          line={s.uninstallLine.replace('{owner}', proposal.owner)}
          reason={proposal.reason}
        />
      );
  }
}
