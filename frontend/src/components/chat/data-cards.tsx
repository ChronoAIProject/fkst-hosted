import { Chip } from '@/components/ui/chip';
import { useContent } from '@/i18n';
import type { DataCard } from './data-card-types';

/**
 * Structured renderings of a tool result.
 *
 * These exist because prose is the worst way to deliver a table. The model can say "you
 * have one environment with two install commands"; the card shows WHICH commands, in
 * order, with the status and the secret names — and it is projected from the response,
 * so it cannot be embellished by generated text.
 *
 * Every card follows the same shape as the rest of the console surface: a HUD frame, a
 * mono label row, and data in a scannable grid. Nothing here is clickable except links
 * that go to GitHub, because a card must never become a second action surface.
 */

type ChatCopy = ReturnType<typeof useContent>['chat'];

/** Panel frame shared by every card. */
function CardFrame({
  label,
  hint,
  children,
  testid,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
  testid: string;
}) {
  return (
    <div
      data-testid={testid}
      className="rounded-card border border-line bg-raise px-2.5 py-2 flex flex-col gap-1.5"
    >
      <div className="flex items-center gap-2 flex-wrap">
        <span className="font-mono text-[10px] font-semibold uppercase tracking-[0.16em] text-amber">
          {label}
        </span>
        {hint != null && hint !== '' && (
          <span className="font-mono text-[10px] text-ghost">{hint}</span>
        )}
      </div>
      {children}
    </div>
  );
}

/** "and N more" footnote. A card that silently truncated would read as complete. */
function Omitted({ count, s }: { count: number; s: ChatCopy }) {
  if (count <= 0) return null;
  return (
    <p data-testid="chat-card-omitted" className="font-mono text-[10px] text-ghost">
      {s.cardOmitted.replace('{count}', String(count))}
    </p>
  );
}

/** Bytes as a compact human size. Log files run from a few hundred bytes to tens of MB,
 *  and raw byte counts are unreadable at both ends. */
function humanBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Environment status → chip tone. The chip TEXT is always the raw status, so meaning
 *  never rides on colour alone. */
function statusTone(status: string): 'green' | 'red' | 'neutral' {
  if (status === 'ready') return 'green';
  if (status === 'failed' || status === 'invalid') return 'red';
  return 'neutral';
}

function EnvironmentsCard({
  card,
  s,
}: {
  card: Extract<DataCard, { kind: 'environments' }>;
  s: ChatCopy;
}) {
  return (
    <CardFrame testid="chat-card-environments" label={s.cardEnvironments}>
      {card.profiles.length === 0 ? (
        <p className="font-mono text-[11px] text-ghost">{s.cardNoEnvironments}</p>
      ) : (
        <ul className="flex flex-col gap-1">
          {card.profiles.map((profile) => (
            <li
              key={profile.name}
              className="flex items-center gap-2 flex-wrap border-l border-l-line-2 pl-2"
            >
              <span className="font-ui text-[12.5px] font-semibold text-fg">{profile.name}</span>
              <Chip tone={statusTone(profile.status)}>{profile.status}</Chip>
              <span className="font-mono text-[10px] text-ghost">
                {s.cardEnvCounts
                  .replace('{install}', String(profile.install_command_count))
                  .replace('{vars}', String(profile.variable_count))
                  .replace('{secrets}', String(profile.secret_count))}
              </span>
            </li>
          ))}
        </ul>
      )}
      <Omitted count={card.omitted} s={s} />
    </CardFrame>
  );
}

function EnvironmentDetailCard({
  card,
  s,
}: {
  card: Extract<DataCard, { kind: 'environment_detail' }>;
  s: ChatCopy;
}) {
  return (
    <CardFrame testid="chat-card-environment-detail" label={s.cardEnvironment} hint={card.name}>
      <div className="flex items-center gap-2">
        <Chip tone={statusTone(card.status)}>{card.status}</Chip>
        {card.validated_at !== '' && (
          <span className="font-mono text-[10px] text-ghost">{card.validated_at}</span>
        )}
      </div>
      {card.install.length > 0 && (
        <pre className="max-h-40 overflow-auto rounded-control border border-line bg-glass px-2.5 py-1.5 font-mono text-[11px] leading-5 text-dim whitespace-pre-wrap">
          {card.install.map((command) => `$ ${command}`).join('\n')}
        </pre>
      )}
      {card.variables.length > 0 && (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 font-mono text-[10.5px]">
          {card.variables.map((variable) => (
            <div key={variable.key} className="contents">
              <dt className="text-ghost">{variable.key}</dt>
              <dd className="text-faint break-words">{variable.value}</dd>
            </div>
          ))}
        </dl>
      )}
      {card.secret_keys.length > 0 && (
        <div className="flex items-center gap-1.5 flex-wrap">
          <span className="font-mono text-[10px] uppercase tracking-[0.1em] text-ghost">
            {s.envSecrets}
          </span>
          {card.secret_keys.map((key) => (
            // Names only, and the value is rendered as a fixed mask — never a
            // placeholder derived from anything real.
            <span
              key={key}
              className="rounded-chip border border-line-2 bg-raise-2 px-1.5 py-0.5 font-mono text-[10px] text-faint"
            >
              {key} <span className="text-ghost">••••</span>
            </span>
          ))}
        </div>
      )}
    </CardFrame>
  );
}

function OutcomesCard({ card, s }: { card: Extract<DataCard, { kind: 'outcomes' }>; s: ChatCopy }) {
  return (
    <CardFrame
      testid="chat-card-outcomes"
      label={s.cardOutcomes}
      hint={`${card.owner}/${card.name} #${card.trigger_issue}`}
    >
      <p className="font-mono text-[10px] text-ghost">
        {s.cardOutcomeSummary
          .replace('{total}', String(card.pull_requests.length))
          .replace('{merged}', String(card.merged))}
      </p>
      <ul className="flex flex-col gap-1">
        {card.pull_requests.map((pr) => (
          <li key={pr.number} className="flex items-center gap-2 flex-wrap">
            <Chip tone={pr.merged ? 'green' : 'neutral'}>
              {pr.merged ? s.cardMerged : pr.state}
            </Chip>
            <a
              href={pr.html_url}
              target="_blank"
              // `noopener` matters on a link built from server data.
              rel="noopener noreferrer"
              data-testid="chat-card-pr-link"
              className="font-ui text-[12px] text-fg no-underline transition-colors hover:text-amber"
            >
              #{pr.number} {pr.title}
            </a>
            <span className="font-mono text-[10px] text-ghost">
              {s.cardFilesChanged.replace('{count}', String(pr.files_changed))}
            </span>
          </li>
        ))}
      </ul>
      <Omitted count={card.omitted} s={s} />
    </CardFrame>
  );
}

function LogRunsCard({ card, s }: { card: Extract<DataCard, { kind: 'log_runs' }>; s: ChatCopy }) {
  return (
    <CardFrame testid="chat-card-log-runs" label={s.cardLogRuns} hint={card.session_id}>
      <ul className="flex flex-col gap-0.5 font-mono text-[10.5px]">
        {card.runs.map((run) => (
          <li key={run.run_id} className="flex items-center gap-2 flex-wrap">
            <span className="text-faint">{run.run_id}</span>
            <span className="text-ghost">{run.started_at}</span>
            {/* A live run has no end time; saying so beats an empty column. */}
            {run.ended_at == null ? (
              <Chip tone="green">{s.cardRunLive}</Chip>
            ) : (
              <span className="text-ghost">→ {run.ended_at}</span>
            )}
          </li>
        ))}
      </ul>
      <Omitted count={card.omitted} s={s} />
    </CardFrame>
  );
}

function LogManifestCard({
  card,
  s,
}: {
  card: Extract<DataCard, { kind: 'log_manifest' }>;
  s: ChatCopy;
}) {
  return (
    <CardFrame
      testid="chat-card-log-manifest"
      label={s.cardLogFiles}
      hint={card.run ?? card.session_id}
    >
      <ul className="flex flex-col gap-0.5 font-mono text-[10.5px]">
        {card.files.map((file) => (
          <li key={file.path} className="flex items-center gap-2">
            <span className="flex-1 truncate text-faint">{file.path}</span>
            <span className="text-ghost">{humanBytes(file.size_bytes)}</span>
          </li>
        ))}
      </ul>
      <Omitted count={card.omitted} s={s} />
    </CardFrame>
  );
}

/** Every card a turn produced, in arrival order. */
export function DataCards({ cards }: { cards: DataCard[] }) {
  const s = useContent().chat;
  if (cards.length === 0) return null;
  return (
    <div data-testid="chat-data-cards" className="flex flex-col gap-1.5">
      {cards.map((card, index) => {
        switch (card.kind) {
          case 'environments':
            return <EnvironmentsCard key={index} card={card} s={s} />;
          case 'environment_detail':
            return <EnvironmentDetailCard key={index} card={card} s={s} />;
          case 'outcomes':
            return <OutcomesCard key={index} card={card} s={s} />;
          case 'log_runs':
            return <LogRunsCard key={index} card={card} s={s} />;
          case 'log_manifest':
            return <LogManifestCard key={index} card={card} s={s} />;
        }
      })}
    </div>
  );
}
