import { useId } from 'react';
import type { SessionDetail } from '@/lib/api/types';
import { DrawerShell } from './drawer-shell';
import { SessionDetailView } from './session-detail-view';

/** The per-session detail drawer: a thin overlay wrapper around the reusable
 *  {@link SessionDetailView}. DrawerShell owns the slide-in chrome, scrim, focus
 *  trap and Escape-to-close; the view owns the sticky header + four-tab body.
 *
 *  The `titleId` generated here is handed to BOTH DrawerShell (as the dialog's
 *  `aria-labelledby` target) and the view (as its heading id), so the dialog is
 *  labelled by the exact heading the view renders. Passing `onClose` down makes
 *  the view render its header Close button; the inline/workspace host omits it. */
export function SessionDetailDrawer({
  owner,
  name,
  session,
  onClose,
}: {
  owner: string;
  name: string;
  session: SessionDetail;
  onClose: () => void;
}) {
  const titleId = useId();

  return (
    <DrawerShell titleId={titleId} onClose={onClose}>
      <SessionDetailView
        owner={owner}
        name={name}
        session={session}
        onClose={onClose}
        titleId={titleId}
      />
    </DrawerShell>
  );
}
