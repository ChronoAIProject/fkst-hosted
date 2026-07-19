import { useLang } from '@/i18n';
import { formatLocal } from '@/lib/format';
import { Chip } from '@/components/ui/chip';
import { StaggerItem } from '@/components/ui/motion';
import type { EnvironmentProfileSummary } from '@/lib/api/types';
import type { EnvManagerStrings } from '@/i18n/en/environments';
import { Note, Spinner, fmt, statusTone, type ListState } from './environments-drawer';

/** One tappable row: name + status chip, validated timestamp, and the three
 *  content counts. Whole row is a button so keyboard users open it with Enter. */
function EnvironmentRow({
  profile,
  index,
  t,
  onOpen,
}: {
  profile: EnvironmentProfileSummary;
  index: number;
  t: EnvManagerStrings;
  onOpen: (name: string) => void;
}) {
  const { lang } = useLang();
  const validated = profile.validated_at
    ? fmt(t.validatedAt, { time: formatLocal(profile.validated_at, lang) })
    : t.neverValidated;

  return (
    <StaggerItem index={index}>
      <button
        type="button"
        onClick={() => onOpen(profile.name)}
        aria-label={fmt(t.openAria, { name: profile.name })}
        className="w-full text-left rounded-card border border-line bg-raise-2 px-3.5 py-3 flex flex-col gap-2 hover:border-line-2 transition-colors cursor-pointer"
      >
        <div className="flex items-center justify-between gap-2">
          <span className="font-mono text-[13px] text-fg truncate">{profile.name}</span>
          <Chip tone={statusTone(profile.status)}>{profile.status}</Chip>
        </div>
        <div className="flex items-center gap-1.5 flex-wrap">
          <Chip tone="neutral">{fmt(t.installCount, { n: profile.install_command_count })}</Chip>
          <Chip tone="neutral">{fmt(t.variableCount, { n: profile.variable_count })}</Chip>
          <Chip tone="neutral">{fmt(t.secretCount, { n: profile.secret_count })}</Chip>
        </div>
        <span className="font-mono text-[11px] text-ghost">{validated}</span>
      </button>
    </StaggerItem>
  );
}

/**
 * The environment list view: a header row with a "New environment" action, then
 * either a loading note, an error note (with retry), an empty state, or the
 * staggered rows. Purely presentational — the fetch + state live in the drawer.
 */
export function EnvironmentList({
  t,
  state,
  onNew,
  onOpen,
  onRetry,
}: {
  t: EnvManagerStrings;
  state: ListState;
  onNew: () => void;
  onOpen: (name: string) => void;
  onRetry: () => void;
}) {
  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center justify-end">
        <button
          type="button"
          onClick={onNew}
          className="font-ui font-semibold text-[12.5px] rounded-control px-3.5 py-2 bg-amber text-amber-ink hover:brightness-[1.06] transition-colors cursor-pointer"
        >
          + {t.newEnvironment}
        </button>
      </div>

      {state.status === 'loading' && (
        <div className="flex items-center gap-2">
          <Spinner />
          <Note>{t.listLoading}</Note>
        </div>
      )}

      {state.status === 'error' && (
        <div className="flex flex-col items-start gap-2">
          <Note>{t.listLoadFailed}</Note>
          <button
            type="button"
            onClick={onRetry}
            className="font-ui font-semibold text-[11.5px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg transition-colors cursor-pointer"
          >
            {t.retry}
          </button>
        </div>
      )}

      {state.status === 'loaded' && state.profiles.length === 0 && (
        <div className="flex flex-col gap-1.5 rounded-card border border-dashed border-line px-4 py-6 text-center">
          <p className="font-ui text-[13px] text-dim">{t.listEmpty}</p>
          <p className="font-mono text-[11.5px] text-ghost">{t.listEmptyHint}</p>
        </div>
      )}

      {state.status === 'loaded' && state.profiles.length > 0 && (
        <ul className="flex flex-col gap-2">
          {state.profiles.map((profile, i) => (
            <li key={profile.name}>
              <EnvironmentRow profile={profile} index={i} t={t} onOpen={onOpen} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
