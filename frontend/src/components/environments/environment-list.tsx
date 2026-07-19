import { useLang } from '@/i18n';
import { formatLocal } from '@/lib/format';
import { Chip } from '@/components/ui/chip';
import { StaggerItem } from '@/components/ui/motion';
import type { EnvironmentProfileSummary } from '@/lib/api/types';
import type { EnvManagerStrings } from '@/i18n/en/environments';
import { Note, Spinner, fmt, statusTone, type ListState } from './environments-drawer';

/** Resting depth + a soft status-matched glow so a profile's health reads at a
 *  glance (mirrors the session-card treatment). `.hover-lift` swaps in the raised
 *  shadow + amber bloom on hover, so this returns only the resting composition. */
function statusGlow(status: string): string {
  switch (statusTone(status)) {
    case 'green':
      return 'shadow-[var(--shadow-2),var(--glow-green)]';
    case 'red':
      return 'shadow-[var(--shadow-2),var(--glow-red)]';
    case 'amber':
      return 'shadow-[var(--shadow-2),var(--glow-amber)]';
    default:
      return 'shadow-2';
  }
}

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
        // Elevated glass card: gradient hairline edge + status-matched resting
        // glow, with a hover-lift into raised depth + amber bloom. The whole row
        // stays a single focusable button (behavior/roles unchanged).
        className={`grad-border hover-lift group w-full text-left rounded-card px-3.5 py-3 flex flex-col gap-2 cursor-pointer ${statusGlow(
          profile.status
        )}`}
      >
        <div className="flex items-center justify-between gap-2">
          <span className="font-display font-semibold text-[13.5px] text-fg truncate group-hover:text-amber transition-colors">
            {profile.name}
          </span>
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
          // Primary CTA: brand gradient fill on amber ink, card depth + amber
          // bloom, a one-shot sheen sweep on mount, brightening on hover.
          className="anim-sheen font-ui font-semibold text-[12.5px] rounded-control px-3.5 py-2 bg-grad-accent text-amber-ink shadow-[var(--shadow-2),var(--glow-amber)] hover:brightness-110 transition-[filter,box-shadow] cursor-pointer"
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
        <div className="anim-row-in flex flex-col items-start gap-2 rounded-card bg-glass backdrop-blur-glass border border-line border-l-2 border-l-red px-3.5 py-3 shadow-[var(--shadow-2),var(--glow-red)]">
          <Note>{t.listLoadFailed}</Note>
          <button
            type="button"
            onClick={onRetry}
            className="font-ui font-semibold text-[11.5px] border border-line rounded-control px-2.5 py-1 text-dim hover:text-fg hover:border-line-2 hover:shadow-glow-amber transition-[color,border-color,box-shadow] cursor-pointer"
          >
            {t.retry}
          </button>
        </div>
      )}

      {state.status === 'loaded' && state.profiles.length === 0 && (
        <div className="anim-row-in flex flex-col gap-1.5 rounded-card border border-dashed border-line-2 bg-glass backdrop-blur-glass px-4 py-7 text-center shadow-1">
          <p className="font-display font-semibold text-[13.5px] text-dim">{t.listEmpty}</p>
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
