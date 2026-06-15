import { useForm } from 'react-hook-form';
import { zodResolver } from '@hookform/resolvers/zod';
import { z } from 'zod';
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DialogClose,
} from '../../components/primitives/dialog';
import { ModalSheet } from '../../components/layout/modal-sheet';
import { useCreatePackage } from '../../lib/hooks/usePackages';
import { toast } from '../../components/primitives/toaster';
import { ApiError } from '../../lib/api/client';
import { NewPackage } from '../../lib/api/types';

interface ParsedFile {
  path: string;
  content: string;
}

// Choice & Rationale: We chose a single combined format inside a <textarea>
// where each file is prefixed by a header line like `--- path: <filepath>`.
// This allows developers to paste multiple files at once, including their relative
// directories and file contents, and validates them cleanly in Zod.
export function parseCombinedFiles(filesRaw: string): ParsedFile[] {
  const parts = filesRaw.split(/^---\s*path:\s*/m);
  const files: ParsedFile[] = [];

  for (const part of parts) {
    const trimmedPart = part.trim();
    if (!trimmedPart) continue;

    // Split into first line (path) and rest (content)
    const newlineIndex = trimmedPart.indexOf('\n');
    if (newlineIndex === -1) {
      // Path only, empty content
      files.push({
        path: trimmedPart,
        content: '',
      });
    } else {
      const path = trimmedPart.slice(0, newlineIndex).trim();
      const content = trimmedPart.slice(newlineIndex + 1);
      files.push({
        path,
        content,
      });
    }
  }

  return files;
}

// Form schema with validation rules required by spec
const formSchema = z
  .object({
    name: z
      .string()
      .min(1, 'Package name is required')
      .regex(
        /^[A-Za-z0-9_-]+$/,
        'Package name must contain only alphanumeric characters, underscores, or hyphens'
      )
      .refine(
        (val) => new TextEncoder().encode(val).length <= 128,
        { message: 'Package name must be 128 bytes or less' }
      ),
    filesRaw: z.string().min(1, 'At least one file is required'),
    composedDepsRaw: z.string().optional(),
  })
  .superRefine((data, ctx) => {
    const files = parseCombinedFiles(data.filesRaw);

    if (files.length === 0) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['filesRaw'],
        message: 'At least one file is required',
      });
      return;
    }

    if (files.length > 256) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['filesRaw'],
        message: 'Maximum of 256 files allowed',
      });
    }

    // Engine entry validation: departments/<d>/main.lua or raisers/<name>.lua
    const hasEntry = files.some(
      (f) =>
        /^departments\/[^/]+\/main\.lua$/.test(f.path) || /^raisers\/[^/]+\.lua$/.test(f.path)
    );

    if (!hasEntry) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['filesRaw'],
        message:
          'Package must contain an engine entry point: departments/<d>/main.lua or raisers/<name>.lua',
      });
    }

    // Validate size per-file <= 1MiB (1024 * 1024 bytes) via TextEncoder
    const encoder = new TextEncoder();
    let totalSize = 0;
    let fileExceedsLimit = false;

    files.forEach((f) => {
      const fileBytes = encoder.encode(f.content).length;
      totalSize += fileBytes;
      if (fileBytes > 1024 * 1024) {
        fileExceedsLimit = true;
      }
    });

    if (fileExceedsLimit) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['filesRaw'],
        message: 'Individual file size exceeds 1MiB limit',
      });
    }

    // Validate total size <= 12MiB (12 * 1024 * 1024 bytes)
    if (totalSize > 12 * 1024 * 1024) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['filesRaw'],
        message: 'Total files size exceeds 12MiB limit',
      });
    }
  });

type FormData = z.infer<typeof formSchema>;

export interface AddPackageModalProps {
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}

export function AddPackageModal({ isOpen, onOpenChange }: AddPackageModalProps) {
  const createMutation = useCreatePackage();

  const {
    register,
    handleSubmit,
    setError,
    reset,
    formState: { errors, isSubmitting },
  } = useForm<FormData>({
    resolver: zodResolver(formSchema),
    defaultValues: {
      name: '',
      filesRaw: '',
      composedDepsRaw: '',
    },
  });

  const onSubmit = async (data: FormData) => {
    const files = parseCombinedFiles(data.filesRaw);
    const composedDeps = data.composedDepsRaw
      ? data.composedDepsRaw
          .split(',')
          .map((d) => d.trim())
          .filter(Boolean)
      : [];

    const payload: NewPackage = {
      name: data.name,
      files,
      composed_deps: composedDeps,
    };

    try {
      await createMutation.mutateAsync(payload);

      toast({
        title: 'Created',
        description: 'Created — composes on next session start',
      });
      reset();
      onOpenChange(false);
    } catch (err) {
      if (err instanceof ApiError) {
        if (err.status === 409) {
          setError('name', {
            type: 'server',
            message: 'name already exists (a revision is a new name)',
          });
        } else {
          setError('root', {
            type: 'server',
            message: err.message || 'Server error occurred',
          });
        }
      } else if (err && typeof err === 'object' && 'status' in err && err.status === 409) {
        setError('name', {
          type: 'server',
          message: 'name already exists (a revision is a new name)',
        });
      } else {
        setError('root', {
          type: 'server',
          message: err instanceof Error ? err.message : 'Network error occurred',
        });
      }
    }
  };

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open) reset();
        onOpenChange(open);
      }}
    >
      <DialogContent className="p-0" showClose={false}>
        <ModalSheet
          title={
            <DialogTitle asChild>
              <span>Add package</span>
            </DialogTitle>
          }
          meta={
            <DialogDescription asChild>
              <span>
                create a package in the hosted store —{' '}
                <span className="font-mono text-[11.5px] text-ghost">POST /api/v1/packages</span>
              </span>
            </DialogDescription>
          }
          closeButtonSlot={
            <DialogClose
              aria-label="Close"
              className="w-[30px] h-[30px] rounded-control border border-line bg-raise-2 text-faint hover:text-fg hover:border-faint flex items-center justify-center cursor-pointer transition-colors focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2"
            >
              <span className="text-[17px] leading-none" aria-hidden="true">✕</span>
            </DialogClose>
          }
          actionLeftSlot={
            <span className="text-[11px] text-ghost leading-normal font-mono select-none">
              Client checks are a subset. The server is the authority; every 400 maps inline.
            </span>
          }
          actionButtonsSlot={
            <>
              <button
                type="button"
                onClick={() => {
                  reset();
                  onOpenChange(false);
                }}
                className="text-[12.5px] font-medium border border-line-2 bg-raise-2 text-dim rounded-control px-3.5 py-2 cursor-pointer hover:border-faint hover:text-fg transition-colors select-none"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={handleSubmit(onSubmit)}
                disabled={isSubmitting}
                className="text-[12.5px] font-semibold text-amber-ink bg-amber border-0 rounded-control px-4 py-2 cursor-pointer hover:brightness-[106%] transition-all disabled:opacity-50 select-none"
              >
                {isSubmitting ? 'Creating...' : 'Create package'}
              </button>
            </>
          }
        >
          <div className="flex flex-col gap-4">
            {errors.root && (
              <div className="bg-red/10 border border-red/30 rounded-control p-3 text-red text-[13px] select-text">
                {errors.root.message}
              </div>
            )}

            {/* Name Field */}
            <div className="flex flex-col gap-1.5 min-w-0">
              <label htmlFor="name-input" className="text-[10px] font-mono font-semibold tracking-[0.13em] uppercase text-ghost select-none">
                Name · unique, create-only
              </label>
              <input
                id="name-input"
                {...register('name')}
                placeholder="e.g. github-devloop"
                className="bg-raise-2 border border-line-2 rounded-control text-fg text-[13px] px-3 py-2.5 outline-none w-full focus:border-faint transition-colors"
              />
              {errors.name && (
                <span className="text-red text-[12px] font-mono select-text">{errors.name.message}</span>
              )}
            </div>

            {/* Files Field */}
            <div className="flex flex-col gap-1.5 min-w-0">
              <label htmlFor="files-textarea" className="text-[10px] font-mono font-semibold tracking-[0.13em] uppercase text-ghost select-none">
                Files · the package root, inline
              </label>
              <textarea
                id="files-textarea"
                {...register('filesRaw')}
                placeholder={'--- path: departments/my-dept/main.lua\n-- Lua code goes here\n\n--- path: utils.lua\n-- More Lua code'}
                className="bg-raise-2 border border-line-2 rounded-control text-fg text-[13px] px-3 py-2.5 outline-none w-full min-h-[120px] resize-y leading-normal font-mono focus:border-faint transition-colors"
              />
              {errors.filesRaw && (
                <span className="text-red text-[12px] font-mono select-text">
                  {errors.filesRaw.message}
                </span>
              )}
            </div>

            {/* Composed Deps Field */}
            <div className="flex flex-col gap-1.5 min-w-0">
              <label htmlFor="composed-deps-input" className="text-[10px] font-mono font-semibold tracking-[0.13em] uppercase text-ghost select-none">
                composed_deps · optional
              </label>
              <input
                id="composed-deps-input"
                {...register('composedDepsRaw')}
                placeholder="github-proxy, consensus"
                className="bg-raise-2 border border-line-2 rounded-control text-fg text-[13px] px-3 py-2.5 outline-none w-full focus:border-faint transition-colors"
              />
              {errors.composedDepsRaw && (
                <span className="text-red text-[12px] font-mono select-text">
                  {errors.composedDepsRaw.message}
                </span>
              )}
            </div>

            {/* Honest Note */}
            <div className="font-mono text-[11px] text-ghost leading-relaxed bg-raise-2 border border-line rounded-control p-3 select-none">
              Stored in the hosted package store as <span className="text-faint">{`{name, files[], composed_deps[]}`}</span> — a duplicate name on create is a <b>409</b>; updates (PUT) and deletes (DELETE) are supported via the API (UI coming soon). A package is loaded <b>once</b> at session start and composed into the one static graph — applying it = <b>stop the session, start a new one</b> (<span className="text-faint">POST /sessions/:id/stop</span> → poll <b>stopped</b> → <span className="text-faint">POST /sessions</span>). Create validates <b>structure only</b> (name <span className="text-faint">[A-Za-z0-9_-]+</span> · ≥1 file · ≤256 files · ≤1&nbsp;MiB/file · ≤12&nbsp;MiB total · must contain <span className="text-faint">departments/*/main.lua</span> or <span className="text-faint">raisers/*.lua</span>); <b>conformance runs at session start</b> — the session passes through <b>validating</b> and lands <b>failed</b> with the error on the session record if it doesn't conform. The source tree is <b>read-only at runtime</b>.
            </div>
          </div>
        </ModalSheet>
      </DialogContent>
    </Dialog>
  );
}
