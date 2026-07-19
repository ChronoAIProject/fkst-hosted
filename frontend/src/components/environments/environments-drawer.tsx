import { useCallback, useEffect, useId, useState } from 'react';
import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';
import { useLang } from '@/i18n';
import { useAuth } from '@/lib/auth/github-auth';
import { FadeSwap } from '@/components/ui/motion';
import { DrawerShell } from '@/components/session-detail/drawer-shell';
import { listEnvironmentProfiles } from '@/lib/api/environments';
import type { EnvironmentProfileSummary, EnvironmentProfileView } from '@/lib/api/types';
import { environmentsManager as enStrings } from '@/i18n/en/environments';
import { environmentsManager as zhStrings } from '@/i18n/zh/environments';
import type { EnvManagerStrings } from '@/i18n/en/environments';
import { EnvironmentList } from './environment-list';
import { EnvironmentEditor } from './environment-editor';
import { EnvironmentDetail } from './environment-detail';

// ---- shared, self-contained UI helpers -------------------------------------
// Kept local to the environments cluster (rather than importing session-detail's
// parts.tsx) so the two clusters stay decoupled; these are trivial presentational
// atoms styled from the same tokens.

/** Indeterminate spinner. `anim-spin` is disabled under prefers-reduced-motion
 *  (see index.css), so it collapses to a static ring for reduced-motion users. */
export function Spinner({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        'anim-spin inline-block w-3 h-3 border border-line-2 border-t-amber rounded-full flex-none',
        className
      )}
    />
  );
}

/** Muted mono note line (loading / empty / hint states). */
export function Note({ children }: { children: ReactNode }) {
  return <p className="font-mono text-[11.5px] text-ghost">{children}</p>;
}

/** Uppercase eyebrow heading a section inside the drawer. */
export function SectionLabel({ children }: { children: ReactNode }) {
  return <span className="font-mono text-eyebrow text-ghost uppercase">{children}</span>;
}

/** Substitute `{key}` placeholders in a template. Missing keys are left as-is so
 *  a template/usage drift is visible rather than silently blanked. */
export function fmt(template: string, vars: Record<string, string | number>): string {
  return template.replace(/\{(\w+)\}/g, (whole, key: string) =>
    key in vars ? String(vars[key]) : whole
  );
}

/** Map a backend status string to a Chip tone. The wire status is a free string,
 *  so match known-good/known-bad prefixes and default to neutral. */
export function statusTone(status: string): 'neutral' | 'green' | 'red' | 'amber' {
  const s = status.toLowerCase();
  if (s.includes('valid') && !s.includes('invalid')) return 'green';
  if (s === 'ready' || s === 'ok' || s === 'active') return 'green';
  if (s.includes('fail') || s.includes('error') || s.includes('invalid')) return 'red';
  if (s.includes('pending') || s.includes('validating')) return 'amber';
  return 'neutral';
}

/** Resolve the manager dictionary for the active language. The manager strings
 *  live outside the composed `SiteContent` catalog (so this file's siblings need
 *  no changes to `types.ts`/`en.ts`/`zh.ts`), hence the direct lang→dict map. */
export function useEnvStrings(): EnvManagerStrings {
  return useLang().lang === 'zh' ? zhStrings : enStrings;
}

// ---- view routing -----------------------------------------------------------

/** Which sub-view the drawer body shows. Editor carries an optional `initial`
 *  view (edit mode) — absent means create mode. */
type View =
  | { kind: 'list' }
  | { kind: 'editor'; initial?: EnvironmentProfileView }
  | { kind: 'detail'; name: string };

/** The list-fetch state, shared with the presentational `EnvironmentList`. */
export type ListState =
  | { status: 'loading' }
  | { status: 'error' }
  | { status: 'loaded'; profiles: EnvironmentProfileSummary[] };

/**
 * User-scoped environment-profile manager. Reuses the session-detail
 * `DrawerShell` for a full-height right drawer and routes between three sub-views
 * (list / editor / detail). The list is fetched here and re-fetched whenever a
 * mutation (save / delete) reports success, so returning to the list always
 * shows fresh counts and statuses.
 */
export function EnvironmentsDrawer({ open, onClose }: { open: boolean; onClose: () => void }) {
  const t = useEnvStrings();
  const { apiFetch } = useAuth();
  const titleId = useId();

  const [view, setView] = useState<View>({ kind: 'list' });
  const [list, setList] = useState<ListState>({ status: 'loading' });

  const loadList = useCallback(() => {
    setList({ status: 'loading' });
    listEnvironmentProfiles(apiFetch)
      .then((profiles) => setList({ status: 'loaded', profiles }))
      .catch(() => setList({ status: 'error' }));
  }, [apiFetch]);

  // Fetch on open and whenever we return to the list view. Resetting to the list
  // view on (re)open keeps the drawer from reopening deep in a stale sub-view.
  useEffect(() => {
    if (!open) return;
    setView({ kind: 'list' });
    loadList();
  }, [open, loadList]);

  const goList = useCallback(() => setView({ kind: 'list' }), []);

  // After a successful save/delete, refresh the list and return to it.
  const onMutated = useCallback(() => {
    loadList();
    setView({ kind: 'list' });
  }, [loadList]);

  const notList = view.kind !== 'list';
  // A stable key per view so FadeSwap crossfades on every navigation.
  const viewKey = view.kind === 'detail' ? `detail:${view.name}` : view.kind;

  return (
    <DrawerShell titleId={titleId} onClose={onClose} open={open}>
      <div className="sticky top-0 z-10 bg-raise border-b border-line px-5 py-4 flex items-center justify-between gap-3">
        <div className="min-w-0 flex items-center gap-2">
          {notList && (
            <button
              type="button"
              onClick={goList}
              aria-label={t.backAria}
              className="font-ui font-semibold text-[12px] border border-line rounded-control px-2.5 py-1.5 text-dim hover:text-fg transition-colors cursor-pointer flex-none"
            >
              ← {t.back}
            </button>
          )}
          <h2 id={titleId} className="font-display font-semibold text-[17px] text-fg truncate">
            {t.title}
          </h2>
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label={t.closeAria}
          className="font-ui font-semibold text-[12px] border border-line rounded-control px-3 py-1.5 text-dim hover:text-fg transition-colors cursor-pointer flex-none"
        >
          {t.close}
        </button>
      </div>

      <div className="px-5 py-4">
        <FadeSwap k={viewKey}>
          {view.kind === 'list' && (
            <EnvironmentList
              t={t}
              state={list}
              onNew={() => setView({ kind: 'editor' })}
              onOpen={(name) => setView({ kind: 'detail', name })}
              onRetry={loadList}
            />
          )}
          {view.kind === 'editor' && (
            <EnvironmentEditor
              t={t}
              initial={view.initial}
              onCancel={goList}
              onSaved={onMutated}
            />
          )}
          {view.kind === 'detail' && (
            <EnvironmentDetail
              t={t}
              name={view.name}
              onEdit={(initial) => setView({ kind: 'editor', initial })}
              onDeleted={onMutated}
            />
          )}
        </FadeSwap>
      </div>
    </DrawerShell>
  );
}
