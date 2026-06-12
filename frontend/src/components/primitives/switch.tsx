import * as React from 'react';
import * as SwitchPrimitive from '@radix-ui/react-switch';
import { cn } from '@/lib/utils';

export const Switch = React.forwardRef<
  React.ElementRef<typeof SwitchPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof SwitchPrimitive.Root>
>((({ className, ...props }, ref) => (
  <SwitchPrimitive.Root
    ref={ref}
    className={cn(
      "peer inline-flex h-[18px] w-[30px] shrink-0 cursor-pointer items-center rounded-full bg-line-2 relative transition-colors focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2 hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-[0.62] data-[disabled]:cursor-not-allowed data-[disabled]:opacity-[0.62] data-[state=checked]:bg-[color-mix(in_oklab,var(--amber)_55%,var(--line))]", /* color-mix matches packages.html .sw.on */
      className
    )}
    {...props}
  >
    <SwitchPrimitive.Thumb
      className={cn(
        "pointer-events-none block h-[14px] w-[14px] rounded-full bg-faint absolute top-[2px] left-[2px] transition-[left,background-color] motion-reduce:transition-none data-[state=checked]:left-[14px] data-[state=checked]:bg-amber"
      )}
    />
  </SwitchPrimitive.Root>
)));
Switch.displayName = SwitchPrimitive.Root.displayName;
