import { useContent } from '@/i18n';
import type { SessionDetail } from '@/lib/api/types';
import { packageRole } from '@/lib/api/derive';
import { Note, SectionLabel } from './parts';
import { ObserveView } from './observe-view';
import type { ObserveState } from './observe-state';

/** Packages tab: each declared package decoded to a friendly role + short
 *  handle, with the full `owner/repo@ref:path` kept in a tooltip / `<code>`.
 *  When the Status tab has already fetched the engine snapshot, the same
 *  per-queue activity is surfaced here too (no second fetch). */
export function TabPackages({
  session,
  observe,
}: {
  session: SessionDetail;
  observe: ObserveState;
}) {
  const d = useContent().dashboard;
  const t = d.detail;

  return (
    <div className="flex flex-col gap-5">
      <section className="flex flex-col gap-2">
        <SectionLabel>
          {d.packages}
          {session.packages.length > 0 && (
            <span className="ml-2 lowercase">· {session.packages.length}</span>
          )}
        </SectionLabel>
        {session.packages.length === 0 ? (
          <Note>{t.packagesNone}</Note>
        ) : (
          <ul className="flex flex-col gap-2">
            {session.packages.map((ref) => {
              const decoded = packageRole(ref);
              return (
                <li
                  key={ref}
                  className="border border-line rounded-card bg-bg px-3 py-2 flex flex-col gap-1 min-w-0"
                >
                  <div className="flex items-baseline gap-2 min-w-0">
                    <span className="font-ui font-semibold text-[13px] text-fg flex-none">
                      {decoded.role}
                    </span>
                    {decoded.role !== decoded.short && (
                      <span className="font-mono text-[11px] text-dim truncate min-w-0">
                        {decoded.short}
                      </span>
                    )}
                  </div>
                  <code
                    title={t.packageRefAria}
                    className="font-mono text-[10.5px] text-ghost break-all"
                  >
                    {ref}
                  </code>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      {observe.status === 'loaded' && (
        <section className="flex flex-col gap-2">
          <SectionLabel>{t.queueActivity}</SectionLabel>
          <ObserveView snapshot={observe.snapshot} />
        </section>
      )}
    </div>
  );
}
