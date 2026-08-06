import { useState } from 'react';
import { useContent } from '@/i18n';
import type { RepoSessionsResponse, SessionDetail } from '@/lib/api/types';
import { ScrollArea } from '@/components/ui/scroll-area';
import { SessionDetailView } from '@/components/session-detail/session-detail-view';
import { RepoWorkflows } from '@/components/workflows/repo-workflows';
import { SessionRail } from './session-rail';

/** The two things a repository has: sessions working its issues, and schedules
 *  firing on a cadence. Scheduled workflows used to live behind a top-level
 *  route whose first act was asking which repository you meant — which is a
 *  question this view has already answered. */
type WorkspaceView = 'sessions' | 'workflows';

/** Stable per-repo identity for a session across polls: the session_id once the
 *  backend has assigned one, else the trigger issue number. Deliberately NOT
 *  positional — a poll that reorders the list must not change a session's
 *  identity, which would swap the selected detail out from under the user. */
export function sessionKey(session: SessionDetail): string {
  return session.session_id ?? `trigger-${session.trigger.number}`;
}

/** Whether `key` names this session, accepting EITHER key form.
 *
 *  A deep link can only carry `trigger-<n>` when it is minted before the session
 *  acquires a `session_id` — which is exactly the case for a chat card offering to
 *  open a session it just proposed. Matching both forms means such a link keeps
 *  working after the session starts and its canonical key changes. */
function matchesSelection(session: SessionDetail, key: string | null): boolean {
  if (!key) return false;
  return key === sessionKey(session) || key === `trigger-${session.trigger.number}`;
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
  viewerLogin,
  readOnly = false,
  initialSelectedKey = null,
  onSelectedKeyChange,
}: {
  owner: string;
  name: string;
  /** Poll payload; null while the first fetch is in flight. */
  data: RepoSessionsResponse | null;
  loadFailed: boolean;
  /** A trigger was created or stopped — the page re-fetches immediately. */
  onChanged: () => void;
  /** Verified viewer login used to scope same-creator collision advice. */
  viewerLogin: string;
  /** Hide user-token mutations for an App-wide cross-account projection. */
  readOnly?: boolean;
  /** A session to select on mount — a deep link's `?session=`. Matched by either
   *  key form (see {@link matchesSelection}); an unknown key falls back to the
   *  default first session. */
  initialSelectedKey?: string | null;
  /** Notified whenever the user selects a session, so the page can reflect it in
   *  the URL. */
  onSelectedKeyChange?: (key: string) => void;
}) {
  const sessions = data?.sessions ?? [];
  // Sessions first: a repository that has no schedules is the common case, and
  // opening on an empty schedule table would read as "nothing here".
  const [view, setView] = useState<WorkspaceView>('sessions');

  // Selection is stored as a session KEY (not an index or object) so it stays
  // stable across the parent's silent polls, each of which delivers a fresh
  // array. A null key means "no explicit choice yet" → default to the first
  // session. If the chosen session vanishes from a later poll the lookup misses
  // and we fall back to the first — the detail pane is never left blank while
  // sessions still exist.
  const [selectedKey, setSelectedKey] = useState<string | null>(initialSelectedKey);
  const selected =
    sessions.length === 0
      ? null
      : (sessions.find((s) => matchesSelection(s, selectedKey)) ?? sessions[0]!);

  // Selecting is the user's action, so it both moves the pane and tells the page,
  // which keeps the URL in step.
  const onSelect = (key: string) => {
    setSelectedKey(key);
    onSelectedKeyChange?.(key);
  };

  return (
    <div data-testid="repo-workspace" className="h-full flex flex-col min-h-0 gap-3">
      <ViewSwitch view={view} onChange={setView} />
      {view === 'workflows' ? (
        <RepoWorkflows owner={owner} name={name} />
      ) : (
        <SessionsView
          owner={owner}
          name={name}
          data={data}
          loadFailed={loadFailed}
          onChanged={onChanged}
          viewerLogin={viewerLogin}
          readOnly={readOnly}
          selected={selected}
          onSelect={onSelect}
        />
      )}
    </div>
  );
}

/** The Sessions | Workflows toggle. A segmented control rather than a nav link:
 *  both views are the SAME repository, so moving between them must not read as
 *  navigating away from it. */
function ViewSwitch({
  view,
  onChange,
}: {
  view: WorkspaceView;
  onChange: (view: WorkspaceView) => void;
}) {
  const c = useContent().dashboard;
  const w = useContent().workflows;
  const tabs: [WorkspaceView, string][] = [
    ['sessions', c.sessionsTab],
    ['workflows', w.nav],
  ];
  return (
    <div
      role="tablist"
      aria-label={c.workspaceViewAria}
      data-testid="workspace-view-switch"
      className="flex-none flex items-center gap-1 rounded-control border border-line bg-raise p-0.5 self-start"
    >
      {tabs.map(([id, label]) => (
        <button
          key={id}
          type="button"
          role="tab"
          aria-selected={view === id}
          onClick={() => onChange(id)}
          className={`font-ui text-[12.5px] rounded-control px-3 py-1 cursor-pointer ${
            view === id ? 'bg-shell text-fg shadow-1' : 'text-ghost hover:text-dim'
          }`}
        >
          {label}
        </button>
      ))}
    </div>
  );
}

/** The original rail + detail body, unchanged, lifted out so the switch above
 *  reads as one decision rather than being threaded through every element. */
function SessionsView({
  owner,
  name,
  data,
  loadFailed,
  onChanged,
  viewerLogin,
  readOnly,
  selected,
  onSelect,
}: {
  owner: string;
  name: string;
  data: RepoSessionsResponse | null;
  loadFailed: boolean;
  onChanged: () => void;
  viewerLogin: string;
  readOnly: boolean;
  selected: SessionDetail | null;
  onSelect: (key: string) => void;
}) {
  const c = useContent().dashboard;
  return (
    <div
      data-testid="sessions-view"
      className="flex-1 flex flex-col md:flex-row min-h-0 gap-4 overflow-y-auto overflow-x-hidden md:overflow-hidden"
    >
      {/* Desktop: a fixed-width rail with independent scrolling. Narrow screens
          stack a bounded rail above the detail so the two panes never force a
          horizontal page overflow; this workspace owns the vertical scroll. */}
      <div
        data-testid="session-rail"
        className="w-full h-[240px] md:w-[300px] md:h-auto flex-none flex flex-col min-h-0"
      >
        <ScrollArea className="pr-1">
          <SessionRail
            owner={owner}
            name={name}
            data={data}
            loadFailed={loadFailed}
            onChanged={onChanged}
            viewerLogin={viewerLogin}
            readOnly={readOnly}
            // Always the EFFECTIVE selection (first by default) so the matching
            // row highlights even before the user has clicked anything.
            selectedKey={selected ? sessionKey(selected) : null}
            onSelect={onSelect}
          />
        </ScrollArea>
      </div>

      {/* Right: the selected session's inline detail, or a placeholder when the
          repo has no sessions. It gets a stable narrow-screen height so its own
          ScrollArea remains usable inside the stacked workspace; desktop lets
          flex sizing fill the available row as before. */}
      <div className="w-full h-[560px] md:w-auto md:h-auto md:flex-1 flex-none min-w-0 flex flex-col min-h-0">
        {selected ? (
          <div
            data-testid="session-detail"
            className="grad-border rounded-card shadow-2 flex flex-1 flex-col min-h-0 overflow-hidden"
          >
            {/* Bounds the height only; SessionDetailView owns its own scrolling
                so the header and tablist stay put while a tab's body scrolls. */}
            <div className="flex min-h-0 flex-1 flex-col">
              {/* Key by session so selecting a different one gives a FRESH
                  detail (resets to the Status tab + re-fetches observe) rather
                  than inheriting the previous session's tab/observe state. */}
              <SessionDetailView
                key={sessionKey(selected)}
                owner={owner}
                name={name}
                session={selected}
                onChanged={onChanged}
                readOnly={readOnly}
              />
            </div>
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
