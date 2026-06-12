import * as React from 'react';
import {
  Toast,
  ToastClose,
  ToastDescription,
  ToastProvider,
  ToastTitle,
  ToastViewport,
  ToastAction,
} from './toast';

export interface ToastData {
  id: string;
  title?: React.ReactNode;
  description?: React.ReactNode;
  action?: {
    text: string;
    onClick: () => void;
  };
  open?: boolean;
}

type Listener = (toasts: ToastData[]) => void;
let listeners: Listener[] = [];
let toasts: ToastData[] = [];

const notify = () => {
  listeners.forEach((listener) => listener(toasts));
};

export const toast = ({
  title,
  description,
  action,
}: Omit<ToastData, 'id' | 'open'>) => {
  const id = Math.random().toString(36).substring(2, 9);
  const newToast: ToastData = {
    id,
    title,
    description,
    action,
    open: true,
  };

  // Keep max 5 toasts in queue
  toasts = [newToast, ...toasts].slice(0, 5);
  notify();

  return {
    id,
    dismiss: () => dismissToast(id),
  };
};

const dismissToast = (id: string) => {
  toasts = toasts.map((t) => (t.id === id ? { ...t, open: false } : t));
  notify();
  // Cleanup after transition
  setTimeout(() => {
    toasts = toasts.filter((t) => t.id !== id);
    notify();
  }, 1000);
};

export const useToast = () => {
  const [activeToasts, setActiveToasts] = React.useState<ToastData[]>(toasts);

  React.useEffect(() => {
    const listener = (newToasts: ToastData[]) => {
      setActiveToasts(newToasts);
    };
    listeners.push(listener);
    return () => {
      listeners = listeners.filter((l) => l !== listener);
    };
  }, []);

  return {
    toasts: activeToasts,
    toast,
    dismiss: dismissToast,
  };
};

export function Toaster() {
  const { toasts, dismiss } = useToast();

  return (
    <ToastProvider>
      {toasts.map(({ id, title, description, action, open }) => (
        <Toast
          key={id}
          open={open}
          onOpenChange={(isOpen) => {
            if (!isOpen) dismiss(id);
          }}
        >
          <div className="grid gap-1">
            {title && <ToastTitle>{title}</ToastTitle>}
            {description && <ToastDescription>{description}</ToastDescription>}
          </div>
          {action && (
            <ToastAction altText={action.text} asChild>
              <button
                onClick={action.onClick}
                className="bg-raise border border-line-2 text-dim hover:text-fg hover:border-faint rounded-control px-3 py-1.5 transition-colors text-[12.5px] cursor-pointer"
              >
                {action.text}
              </button>
            </ToastAction>
          )}
          <ToastClose />
        </Toast>
      ))}
      <ToastViewport />
    </ToastProvider>
  );
}
