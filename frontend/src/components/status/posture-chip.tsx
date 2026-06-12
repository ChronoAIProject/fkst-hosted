import React from 'react';
import { cn } from '@/lib/utils';

export interface PostureChipProps {
  className?: string;
}

/**
 * PostureChip component
 * 
 * CRITICAL v1 LAW: It can ONLY render "posture unknown (deploy-time)".
 * This component accepts NO prop that could make it assert REAL or DRY-RUN.
 * Making invalid states unrepresentable prevents configuration errors.
 * 
 * Code comment citing PM-PLAN §1.1 / the missing posture endpoint:
 * The v1 API has no posture endpoint; global FKST_GITHUB_WRITE is a deploy-time env today.
 * Switch to a dynamic control when the hosted config endpoint lands (see PM-PLAN §1.1).
 * The red REAL styling exists in packages.html .posture but is NOT implemented in v1 (v2 work).
 */
export const PostureChip: React.FC<PostureChipProps> = ({ className }) => {
  return (
    <div
      className={cn(
        'inline-flex items-center gap-[7px] font-ui font-semibold text-[11.5px] tracking-[0.02em] px-[10px] py-[5px] rounded-[8px]',
        'bg-raise border border-line text-faint',
        className
      )}
    >
      <span className="h-[7px] w-[7px] rounded-full bg-ghost flex-shrink-0" aria-hidden="true" />
      <span>posture unknown (deploy-time)</span>
    </div>
  );
};
