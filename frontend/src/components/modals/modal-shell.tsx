import { useEffect } from 'react';
import type { ReactNode } from 'react';

/** Shared modal chrome: dimmed overlay, centered dialog card, Escape-to-close.
 *  Callers provide the heading (labelling the dialog) and the body content.
 *  The Escape handler stops propagation so page-level Escape shortcuts (the
 *  canvas back affordance) never fire while a dialog is open. */
export function ModalShell({
  titleId,
  title,
  onClose,
  children,
}: {
  titleId: string;
  title: string;
  onClose: () => void;
  children: ReactNode;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onClose();
      }
    };
    // Capture phase so the dialog wins over any window-level Escape listener.
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [onClose]);

  return (
    <div className="anim-overlay-in fixed inset-0 z-50 flex items-center justify-center p-4 bg-[color-mix(in_oklab,var(--bg)_72%,transparent)]">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="anim-modal-in w-full max-w-[460px] border border-line rounded-modal bg-raise shadow-modal-seat p-6 max-[600px]:p-5 max-h-[85vh] overflow-y-auto"
      >
        <div className="flex flex-col gap-4">
          <h3 id={titleId} className="font-display font-semibold text-modal-title text-fg">
            {title}
          </h3>
          {children}
        </div>
      </div>
    </div>
  );
}
