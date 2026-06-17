// presentational; behavior (role, focus, Esc, backdrop) comes from the primitives Dialog.

import { cn } from '@/lib/utils';

export interface ModalSheetProps extends Omit<React.HTMLAttributes<HTMLDivElement>, 'title'> {
  title: React.ReactNode;
  meta?: React.ReactNode;
  closeButtonSlot?: React.ReactNode;
  children?: React.ReactNode; // Represents the body content
  actionLeftSlot?: React.ReactNode;
  actionButtonsSlot?: React.ReactNode;
}

export function ModalSheet({
  title,
  meta,
  closeButtonSlot,
  children,
  actionLeftSlot,
  actionButtonsSlot,
  className,
  ...props
}: ModalSheetProps) {
  return (
    <div
      className={cn(
        'bg-raise border border-line-2 rounded-modal shadow-modal-seat w-full max-w-[560px] flex flex-col overflow-hidden',
        className
      )}
      {...props}
    >
      {/* Header zone */}
      <div className="py-5 px-[22px] border-b border-line flex items-start justify-between gap-4">
        <div className="flex flex-col min-w-0">
          <h2 className="text-modal-title font-semibold text-fg font-display leading-tight truncate">
            {title}
          </h2>
          {meta && (
            <div className="font-mono text-[11.5px] text-ghost truncate mt-1">
              {meta}
            </div>
          )}
        </div>
        {closeButtonSlot && (
          <div className="flex-none -mt-1 -mr-1">
            {closeButtonSlot}
          </div>
        )}
      </div>

      {/* Body zone */}
      <div className="px-[22px] py-4 flex-1 text-body text-fg overflow-y-auto min-w-0">
        {children}
      </div>

      {/* Action-bar zone */}
      {(actionLeftSlot || actionButtonsSlot) && (
        <div className="border-t border-line px-[22px] py-4 flex items-center justify-between gap-4">
          <div className="min-w-0 flex items-center gap-2">
            {actionLeftSlot}
          </div>
          <div className="flex-none flex items-center gap-2 ml-auto">
            {actionButtonsSlot}
          </div>
        </div>
      )}
    </div>
  );
}
