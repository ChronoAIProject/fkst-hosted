import { AnimatePresence, motion, useReducedMotion } from 'framer-motion';
import type { ReactNode } from 'react';
import { useContent } from '@/i18n';
import { levelKey } from '@/components/canvas/level';
import type { CanvasLevel } from '@/components/canvas/level';

/** The level-aware right sidebar shell. Content slides/fades when the level
 *  changes (plain fade under prefers-reduced-motion); within a level, data
 *  refreshes swap in place with no re-animation. */
export function SidebarPanel({ level, children }: { level: CanvasLevel; children: ReactNode }) {
  const cc = useContent().dashboard.canvas;
  const reduce = useReducedMotion();
  const slide = reduce ? 0 : 20;

  return (
    <aside
      aria-label={cc.sidebarAria}
      className="w-[400px] max-[1100px]:w-full flex-none border border-line rounded-panel bg-raise p-5 overflow-y-auto max-h-[720px] max-[1100px]:max-h-none"
    >
      <AnimatePresence mode="wait" initial={false}>
        <motion.div
          key={levelKey(level)}
          initial={{ opacity: 0, x: slide }}
          animate={{ opacity: 1, x: 0 }}
          exit={{ opacity: 0, x: -slide }}
          transition={{ duration: 0.2, ease: 'easeOut' }}
        >
          {children}
        </motion.div>
      </AnimatePresence>
    </aside>
  );
}
