import { useState } from 'react';
import { useContent } from '@/i18n';
import type { RepoSessionsResponse, SessionDetail } from '@/lib/api/types';
import { ScrollArea } from '@/components/ui/scroll-area';
import { SessionDetailView } from '@/components/session-detail/session-detail-view';
import { SessionRail } from './session-rail';

/** Stable per-repo identity for a session across polls: the session_id once the
 *  backend has assigned one, else the trigger issue number. Deliberately NOT
 *  positional — a poll that reorders the list must not change a session's
 *  identity, which would swap the selected detail out from under the user. */
export function sessionKey(session: SessionDetail): string {
  return session.session_id ?? `trigger-${session.trigger.number}`;
}

/** The repo-details workspace: a full-width level-2 view that replaces the
 *  cramped sidebar + the redundant session graph. A left RAIL lists the repo's
 *  sessions (each a compact, selectable card) inside its own bounded scroll
 *  region; the right area renders the selected session's detail INLINE (the same
 *  {@link SessionDetailView} the drawer wraps, minus its Close button) in a
 *  second scroll region. Same prop contract as the former Level2Sidebar, so the
 *  dashboard can swap one for the other. */
export function RepoWorkspace({
  owner,
  name,
  data,
  loadFailed,
  onChanged,
  readOnly = false,
}: {
  owner: string;
  name: string;
  /** Poll payload; null while the first fetch is in flight. */
  data: RepoSessionsResponse | null;
  loadFailed: boolean;
  /** A trigger was created or stopped — the page re-fetches immediately. */
  onChanged: () => void;
  /** Hide user-token mutations for an App-wide cross-account projection. */
  readOnly?: boolean;
}) {
  const c = useContent().dashboard;
  const sessions = data?.sessions ?? [];

  // Selection is stored as a session KEY (not an index or object) so it stays
  // stable across the parent's silent polls, each of which delivers a fresh
  // array. A null key means "no explicit choice yet" → default to the first
  // session. If the chosen session vanishes from a later poll the lookup misses
  // and we fall back to the first — the detail pane is never left blank while
  // sessions still exist.
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const selected =
    sessions.length === 0
      ? null
      : (sessions.find((s) => sessionKey(s) === selectedKey) ?? sessions[0]!);

  return (
    <div className="h-full flex min-h-0 gap-4">
      {/* Left: the session rail — a fixed-width column with its own bounded
          scroll region so a long session list scrolls independently of the
          detail pane. */}
      <div data-testid="session-rail" className="w-[300px] flex-none flex flex-col min-h-0">
        <ScrollArea className="pr-1">
          <SessionRail
            owner={owner}
            name={name}
            data={data}
            loadFailed={loadFailed}
            onChanged={onChanged}
            readOnly={readOnly}
            // Always the EFFECTIVE selection (first by default) so the matching
            // row highlights even before the user has clicked anything.
            selectedKey={selected ? sessionKey(selected) : null}
            onSelect={setSelectedKey}
          />
        </ScrollArea>
      </div>

      {/* Right: the selected session's inline detail, or a placeholder when the
          repo has no sessions. The elevated panel clips its rounded corners; the
          inner ScrollArea is the scroll parent the detail's sticky header pins
          against. */}
      <div className="flex-1 min-w-0 flex flex-col min-h-0">
        {selected ? (
          <div
            data-testid="session-detail"
            className="grad-border rounded-card shadow-2 flex flex-1 flex-col min-h-0 overflow-hidden"
          >
            <ScrollArea>
              {/* Key by session so selecting a different one gives a FRESH
                  detail (resets to the Status tab + re-fetches observe) rather
                  than inheriting the previous session's tab/observe state. */}
              <SessionDetailView
                key={sessionKey(selected)}
                owner={owner}
                name={name}
                session={selected}
              />
            </ScrollArea>
          </div>
        ) : (
          <div className="grad-border rounded-card shadow-2 flex flex-1 items-center justify-center p-8">
            <p className="font-mono text-[12.5px] text-ghost italic text-center">{c.noSessions}</p>
          </div>
        )}
      </div>
    </div>
  );
}
