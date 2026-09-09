import { Link } from 'react-router-dom';
import { Chip } from '@/components/ui/chip';
import { useContent } from '@/i18n';
import type { SessionRef } from './transport';

/** Cap on rendered cards. A turn that touched many sessions should surface the
 *  first few, not bury the answer under a wall of cards. */
const MAX_CARDS = 6;

type ChipTone = 'neutral' | 'green' | 'red';

/**
 * Raw reconciler label → chip tone.
 *
 * A LOCAL table on purpose: `PHASE_TONE`/`WORK_TONE` are keyed by DECODED phases
 * and need a full `SessionDetail` to compute, which a `SessionRef` is not. Trying
 * to index them with a raw label would silently miss every time.
 *
 * The chip's TEXT is always the raw label, so meaning never rides on colour.
 */
export function labelTone(label?: string): ChipTone {
  switch (label) {
    case 'fkst-substrate-active':
    case 'fkst-picked-up':
      return 'green';
    case 'fkst-degraded':
    case 'fkst-substrate-invalid':
    case 'fkst-config-rejected':
    case 'fkst-trigger-unauthorized':
    case 'fkst-unrouted':
    case 'fkst-unauthorized':
      return 'red';
    // A retired session is a fact, not a problem, so it reads neutral — as does
    // anything unrecognized, because guessing a tone would mislead.
    case 'fkst-session-retired':
    default:
      return 'neutral';
  }
}

/** The dashboard key for a session. Matches `RepoWorkspace`'s `sessionKey`, and the
 *  `trigger-<n>` fallback is what makes a card minted before the session has an id
 *  still resolve — the workspace accepts both forms. */
function sessionKey(ref: SessionRef): string {
  return ref.session_id ?? `trigger-${ref.trigger_number}`;
}

/**
 * Deep-linking cards for the sessions a turn identified.
 *
 * They derive ONLY from `message.sessionRefs`, which the backend populated from
 * structured tool results. The model's prose is never parsed for card data: a card
 * that navigates the user somewhere must not be steerable by generated text.
 */
export function RichCards({ refs }: { refs: SessionRef[] }) {
  const s = useContent().chat;
  if (refs.length === 0) return null;

  return (
    <div data-testid="chat-rich-cards" className="flex flex-col gap-1.5">
      {refs.slice(0, MAX_CARDS).map((ref) => {
        const key = sessionKey(ref);
        const label = ref.status_label;
        return (
          <div
            key={`${ref.owner}/${ref.name}#${ref.trigger_number}`}
            data-testid="chat-session-card"
            className="rounded-card border border-line bg-raise px-2.5 py-2 flex flex-col gap-1.5"
          >
            <div className="flex items-center gap-2 flex-wrap">
              <span className="font-ui text-[12.5px] font-semibold text-fg">
                {ref.title ?? `${s.triggerPrefix}${ref.trigger_number}`}
              </span>
              <span className="font-mono text-[10.5px] text-ghost">
                {ref.owner}/{ref.name}
              </span>
              <span className="flex-1" aria-hidden="true" />
              {/* The raw label IS the chip text, so the tone only reinforces it. */}
              <Chip tone={labelTone(label)}>{label ?? s.sessionChip}</Chip>
            </div>
            <div className="flex items-center gap-1.5 flex-wrap">
              <Link
                to={`/dashboard?owner=${encodeURIComponent(ref.owner)}&repo=${encodeURIComponent(
                  ref.name
                )}&session=${encodeURIComponent(key)}`}
                data-testid="chat-card-dashboard-link"
                className="rounded-chip border border-line-2 bg-raise-2 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.1em] text-faint no-underline transition-colors hover:border-[color-mix(in_oklab,var(--amber)_40%,var(--line-2))] hover:text-fg"
              >
                {s.openInDashboard}
              </Link>
              <a
                href={`https://github.com/${ref.owner}/${ref.name}/issues/${ref.trigger_number}`}
                target="_blank"
                // `noopener` matters on a link built from server data: without it the
                // opened page gets a handle on this window.
                rel="noopener noreferrer"
                data-testid="chat-card-trigger-link"
                className="rounded-chip border border-line-2 bg-raise-2 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.1em] text-faint no-underline transition-colors hover:border-[color-mix(in_oklab,var(--amber)_40%,var(--line-2))] hover:text-fg"
              >
                {s.openTrigger} ↗
              </a>
            </div>
          </div>
        );
      })}
    </div>
  );
}
