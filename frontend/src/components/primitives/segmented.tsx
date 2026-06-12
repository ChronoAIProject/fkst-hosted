import * as React from 'react';
import * as TabsPrimitive from '@radix-ui/react-tabs';
import { cn } from '@/lib/utils';

export const Segmented = TabsPrimitive.Root;

export const SegmentedList = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.List>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.List>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.List
    ref={ref}
    className={cn(
      "inline-flex items-center justify-center rounded-[9px] bg-raise border border-line p-[2px] text-faint",
      className
    )}
    {...props}
  />
));
SegmentedList.displayName = TabsPrimitive.List.displayName;

export const SegmentedTrigger = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Trigger>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Trigger
    ref={ref}
    className={cn(
      "inline-flex items-center justify-center whitespace-nowrap rounded-control px-[13px] py-[5px] text-[12.5px] font-medium transition-colors cursor-pointer select-none focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2 hover:text-dim hover:bg-[color-mix(in_oklab,var(--raise-2)_65%,transparent)] data-[state=active]:bg-amber data-[state=active]:text-amber-ink data-[state=active]:font-semibold disabled:pointer-events-none disabled:opacity-[0.62]",
      className
    )}
    {...props}
  />
));
SegmentedTrigger.displayName = TabsPrimitive.Trigger.displayName;

export const SegmentedContent = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Content>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Content
    ref={ref}
    className={cn("focus-visible:outline-none", className)}
    {...props}
  />
));
SegmentedContent.displayName = TabsPrimitive.Content.displayName;
