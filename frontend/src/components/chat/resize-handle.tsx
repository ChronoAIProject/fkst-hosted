/* eslint-disable jsx-a11y/no-noninteractive-element-interactions, jsx-a11y/no-noninteractive-tabindex --
 * A focusable `separator` carrying `aria-valuenow` IS a widget in ARIA: the
 * window-splitter pattern, which is precisely what this file implements.
 * jsx-a11y treats `separator` as non-interactive regardless of focusability, so
 * both rules are false positives here. The scope is this file, which contains
 * nothing but that splitter. Switching to `role="slider"` would silence the
 * linter but announce a pane divider as a value input, which is worse for the
 * screen-reader user this rule exists to protect.
 */
import { useCallback, useEffect, useRef } from 'react';
import { useContent } from '@/i18n';
import { MAX_WIDTH, MIN_WIDTH, RESIZE_STEP } from './use-window-state';

/**
 * The panel's drag-to-resize edge.
 *
 * On the INNER (left) edge, because the panel is docked right: dragging left
 * widens it, which is the direction the panel actually grows.
 *
 * Exposed as a `separator` with `aria-valuenow` and arrow-key handling, so the
 * width is adjustable without a pointer — a drag-only affordance would put the
 * whole feature out of reach of keyboard users.
 *
 * Pointer capture is what makes the drag survive the cursor leaving the 6px
 * strip; without it a fast drag drops the gesture as soon as it outruns the
 * element.
 */
export function ResizeHandle({
  width,
  onResize,
  disabled,
}: {
  width: number;
  onResize: (next: number) => void;
  disabled?: boolean;
}) {
  const s = useContent().chat;
  const draggingRef = useRef(false);

  const onPointerDown = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (disabled) return;
      draggingRef.current = true;
      event.currentTarget.setPointerCapture(event.pointerId);
      event.preventDefault();
    },
    [disabled]
  );

  const onPointerMove = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (!draggingRef.current) return;
      // Width is measured from the RIGHT edge of the viewport, because the panel is
      // right-docked: the pointer's distance from that edge IS the new width.
      onResize(window.innerWidth - event.clientX);
    },
    [onResize]
  );

  const endDrag = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (!draggingRef.current) return;
    draggingRef.current = false;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  }, []);

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (disabled) return;
      // Left widens and right narrows, matching the drag direction rather than the
      // arrow's literal screen direction.
      if (event.key === 'ArrowLeft') {
        event.preventDefault();
        onResize(width + RESIZE_STEP);
      } else if (event.key === 'ArrowRight') {
        event.preventDefault();
        onResize(width - RESIZE_STEP);
      }
    },
    [disabled, onResize, width]
  );

  // A drag that ends outside the window never fires the element's pointerup, so the
  // ref would stay stuck true and the next pointermove would resize unbidden.
  useEffect(() => {
    const stop = () => {
      draggingRef.current = false;
    };
    window.addEventListener('pointerup', stop);
    window.addEventListener('pointercancel', stop);
    return () => {
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('pointercancel', stop);
    };
  }, []);

  if (disabled) return null;

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={s.resizeAria}
      aria-valuenow={width}
      aria-valuemin={MIN_WIDTH}
      aria-valuemax={MAX_WIDTH}
      tabIndex={0}
      data-testid="chat-resize"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onKeyDown={onKeyDown}
      className="absolute inset-y-0 left-0 z-20 w-1.5 cursor-ew-resize transition-colors hover:bg-[color-mix(in_oklab,var(--amber)_35%,transparent)] focus-visible:bg-[color-mix(in_oklab,var(--amber)_45%,transparent)]"
    />
  );
}
