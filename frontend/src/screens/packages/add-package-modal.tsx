import { useForm } from 'react-hook-form';
import { zodResolver } from '@hookform/resolvers/zod';
import { z } from 'zod';
import { useState, useEffect } from 'react';
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
import { NewPackage, PackageFile } from '../../lib/api/types';
import {
  useUpdatePackage,
  useArchiveCreate,
  useGeneratePackage,
} from '../../lib/hooks/usePackageMutations';
import {
  Segmented,
  SegmentedList,
  SegmentedTrigger,
} from '../../components/primitives/segmented';
import { Switch } from '../../components/primitives/switch';

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

function getErrorMessage(err: unknown): string {
  if (err instanceof ApiError) {
    if (err.status === 403) {
      return 'Action forbidden: you do not have permission (403)';
    }
    if (err.status === 404) {
      return 'Action failed: package not found (404)';
    }
    if (err.status === 409) {
      return 'Action failed: package already exists or is in use (409)';
    }
    if (err.status === 503) {
      return 'AI Generation failed: LLM gateway is not configured (503)';
    }
    return err.message || `Request failed with status ${err.status}`;
  }
  const status = err && typeof err === 'object' && ('status' in err || 'statusCode' in err)
    ? (err as { status?: number; statusCode?: number }).status || (err as { status?: number; statusCode?: number }).statusCode
    : undefined;
  const message = err && typeof err === 'object' && 'message' in err
    ? (err as { message?: string }).message
    : undefined;

  if (status === 403) {
    return 'Action forbidden: you do not have permission (403)';
  }
  if (status === 404) {
    return 'Action failed: package not found (404)';
  }
  if (status === 409) {
    return 'Action failed: package already exists or is in use (409)';
  }
  if (status === 503) {
    return 'AI Generation failed: LLM gateway is not configured (503)';
  }
  return message || (err instanceof Error ? err.message : 'An unexpected error occurred');
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
  mode?: 'create' | 'update';
  packageName?: string;
  initialFiles?: PackageFile[];
  initialComposedDeps?: string[];
}

export function AddPackageModal({
  isOpen,
  onOpenChange,
  mode = 'create',
  packageName,
  initialFiles,
  initialComposedDeps,
}: AddPackageModalProps) {
  const createMutation = useCreatePackage();
  const updateMutation = useUpdatePackage();
  const archiveCreateMutation = useArchiveCreate();
  const generateMutation = useGeneratePackage();

  const [activeTab, setActiveTab] = useState<'files' | 'zip' | 'ai'>('files');

  // Zip tab states
  const [zipName, setZipName] = useState('');
  const [zipFile, setZipFile] = useState<File | null>(null);
  const [isZipSubmitting, setIsZipSubmitting] = useState(false);
  const [zipError, setZipError] = useState<string | null>(null);

  // AI tab states
  const [aiName, setAiName] = useState('');
  const [aiDescription, setAiDescription] = useState('');
  const [aiSave, setAiSave] = useState(true);
  const [isAiSubmitting, setIsAiSubmitting] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);

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

  // Sync initial values when modal opens in edit/update mode
  useEffect(() => {
    if (isOpen) {
      if (mode === 'update' && packageName) {
        const filesRaw = initialFiles
          ? initialFiles.map((f) => `--- path: ${f.path}\n${f.content}`).join('\n\n')
          : '';
        const composedDepsRaw = initialComposedDeps
          ? initialComposedDeps.join(', ')
          : '';
        reset({
          name: packageName,
          filesRaw,
          composedDepsRaw,
        });
        setActiveTab('files');
      } else {
        reset({
          name: '',
          filesRaw: '',
          composedDepsRaw: '',
        });
        setActiveTab('files');
        setZipName('');
        setZipFile(null);
        setZipError(null);
        setAiName('');
        setAiDescription('');
        setAiSave(true);
        setAiError(null);
      }
    }
  }, [isOpen, mode, packageName, initialFiles, initialComposedDeps, reset]);

  const onSubmitFiles = async (data: FormData) => {
    const files = parseCombinedFiles(data.filesRaw);
    const composedDeps = data.composedDepsRaw
      ? data.composedDepsRaw
          .split(',')
          .map((d) => d.trim())
          .filter(Boolean)
      : [];

    try {
      if (mode === 'update' && packageName) {
        await updateMutation.mutateAsync({
          name: packageName,
          pkg: {
            files,
            composed_deps: composedDeps,
          },
        });
        toast({
          title: 'Updated',
          description: 'Updated — composes on next session start',
        });
      } else {
        const payload: NewPackage = {
          name: data.name,
          files,
          composed_deps: composedDeps,
        };
        await createMutation.mutateAsync(payload);
        toast({
          title: 'Created',
          description: 'Created — composes on next session start',
        });
      }
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
            message: getErrorMessage(err),
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
          message: getErrorMessage(err),
        });
      }
    }
  };

  const onSubmitZip = async () => {
    setZipError(null);
    if (!zipName.trim()) {
      setZipError('Package name is required');
      return;
    }
    if (!/^[A-Za-z0-9_-]+$/.test(zipName)) {
      setZipError('Package name must contain only alphanumeric characters, underscores, or hyphens');
      return;
    }
    if (new TextEncoder().encode(zipName).length > 128) {
      setZipError('Package name must be 128 bytes or less');
      return;
    }
    if (!zipFile) {
      setZipError('Please select a ZIP file');
      return;
    }

    setIsZipSubmitting(true);
    try {
      const reader = new FileReader();
      const zipBytes = await new Promise<ArrayBuffer>((resolve, reject) => {
        reader.onload = () => resolve(reader.result as ArrayBuffer);
        reader.onerror = () => reject(new Error('Failed to read ZIP file'));
        reader.readAsArrayBuffer(zipFile);
      });
      await archiveCreateMutation.mutateAsync({ name: zipName.trim(), zipBytes });
      toast({
        title: 'Uploaded',
        description: 'Uploaded — composes on next session start',
      });
      setZipName('');
      setZipFile(null);
      onOpenChange(false);
    } catch (err) {
      setZipError(getErrorMessage(err));
    } finally {
      setIsZipSubmitting(false);
    }
  };

  const onSubmitAi = async () => {
    setAiError(null);
    if (!aiDescription.trim()) {
      setAiError('Description is required');
      return;
    }

    setIsAiSubmitting(true);
    try {
      const response = await generateMutation.mutateAsync({
        description: aiDescription.trim(),
        name: aiName.trim() || undefined,
        save: aiSave,
      });

      if (aiSave) {
        if (response.saved) {
          toast({
            title: 'Generated',
            description: 'AI Generated and saved to store — composes on next session start',
          });
        } else {
          setAiError(response.save_error || 'Failed to save generated package');
          setIsAiSubmitting(false);
          return;
        }
      } else {
        toast({
          title: 'Generated',
          description: 'AI Generated successfully (not saved)',
        });
      }

      setAiName('');
      setAiDescription('');
      onOpenChange(false);
    } catch (err) {
      setAiError(getErrorMessage(err));
    } finally {
      setIsAiSubmitting(false);
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
              <span>{mode === 'update' ? 'Update package' : 'Add package'}</span>
            </DialogTitle>
          }
          meta={
            <DialogDescription asChild>
              {mode === 'update' ? (
                <span>
                  update an existing package —{' '}
                  <span className="font-mono text-[11.5px] text-ghost">PUT /api/v1/packages/{packageName}</span>
                </span>
              ) : (
                <span>
                  create a package in the hosted store —{' '}
                  <span className="font-mono text-[11.5px] text-ghost">POST /api/v1/packages</span>
                </span>
              )
            }
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
                  setZipName('');
                  setZipFile(null);
                  setAiName('');
                  setAiDescription('');
                  onOpenChange(false);
                }}
                className="text-[12.5px] font-medium border border-line-2 bg-raise-2 text-dim rounded-control px-3.5 py-2 cursor-pointer hover:border-faint hover:text-fg transition-colors select-none"
              >
                Cancel
              </button>
              {activeTab === 'files' && (
                <button
                  type="button"
                  onClick={handleSubmit(onSubmitFiles)}
                  disabled={isSubmitting}
                  className="text-[12.5px] font-semibold text-amber-ink bg-amber border-0 rounded-control px-4 py-2 cursor-pointer hover:brightness-[106%] transition-all disabled:opacity-50 select-none"
                >
                  {isSubmitting ? 'Submitting...' : mode === 'update' ? 'Update package' : 'Create package'}
                </button>
              )}
              {activeTab === 'zip' && (
                <button
                  type="button"
                  onClick={onSubmitZip}
                  disabled={isZipSubmitting}
                  className="text-[12.5px] font-semibold text-amber-ink bg-amber border-0 rounded-control px-4 py-2 cursor-pointer hover:brightness-[106%] transition-all disabled:opacity-50 select-none"
                >
                  {isZipSubmitting ? 'Uploading...' : 'Upload archive'}
                </button>
              )}
              {activeTab === 'ai' && (
                <button
                  type="button"
                  onClick={onSubmitAi}
                  disabled={isAiSubmitting}
                  className="text-[12.5px] font-semibold text-amber-ink bg-amber border-0 rounded-control px-4 py-2 cursor-pointer hover:brightness-[106%] transition-all disabled:opacity-50 select-none"
                >
                  {isAiSubmitting ? 'Generating...' : 'Generate package'}
                </button>
              )}
            </>
          }
        >
          <div className="flex flex-col gap-4">
            {mode === 'create' && (
              <Segmented value={activeTab} onValueChange={(v) => setActiveTab(v as 'files' | 'zip' | 'ai')} className="w-full">
                <SegmentedList className="w-full grid grid-cols-3">
                  <SegmentedTrigger value="files">Paste files</SegmentedTrigger>
                  <SegmentedTrigger value="zip">Upload .zip</SegmentedTrigger>
                  <SegmentedTrigger value="ai">Generate with AI</SegmentedTrigger>
                </SegmentedList>
              </Segmented>
            )}

            {/* TAB CONTENT: Paste files */}
            {activeTab === 'files' && (
              <div className="flex flex-col gap-4">
                {errors.root && (
                  <div className="bg-red/10 border border-red/30 rounded-control p-3 text-red text-[13px] select-text">
                    {errors.root.message}
                  </div>
                )}

                {/* Name Field */}
                <div className="flex flex-col gap-1.5 min-w-0">
                  <label htmlFor="name-input" className="text-[10px] font-mono font-semibold tracking-[0.13em] uppercase text-ghost select-none">
                    Name · unique on create
                  </label>
                  <input
                    id="name-input"
                    {...register('name')}
                    disabled={mode === 'update'}
                    placeholder="e.g. github-devloop"
                    className="bg-raise-2 border border-line-2 rounded-control text-fg text-[13px] px-3 py-2.5 outline-none w-full focus:border-faint transition-colors disabled:opacity-50"
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
              </div>
            )}

            {/* TAB CONTENT: Upload zip */}
            {activeTab === 'zip' && (
              <div className="flex flex-col gap-4">
                {zipError && (
                  <div className="bg-red/10 border border-red/30 rounded-control p-3 text-red text-[13px] font-mono select-text" role="alert">
                    {zipError}
                  </div>
                )}

                <div className="flex flex-col gap-1.5 min-w-0">
                  <label htmlFor="zip-name-input" className="text-[10px] font-mono font-semibold tracking-[0.13em] uppercase text-ghost select-none">
                    Name · unique on upload
                  </label>
                  <input
                    id="zip-name-input"
                    value={zipName}
                    onChange={(e) => setZipName(e.target.value)}
                    placeholder="e.g. my-zipped-package"
                    className="bg-raise-2 border border-line-2 rounded-control text-fg text-[13px] px-3 py-2.5 outline-none w-full focus:border-faint transition-colors"
                  />
                </div>

                <div className="flex flex-col gap-1.5 min-w-0">
                  <label htmlFor="zip-file-input" className="text-[10px] font-mono font-semibold tracking-[0.13em] uppercase text-ghost select-none">
                    ZIP File · raw application/zip
                  </label>
                  <div className="relative border border-dashed border-line-2 hover:border-faint rounded-control p-6 bg-raise-2 transition-colors flex flex-col items-center justify-center cursor-pointer">
                    <input
                      id="zip-file-input"
                      type="file"
                      accept=".zip"
                      onChange={(e) => {
                        const file = e.target.files?.[0] || null;
                        setZipFile(file);
                      }}
                      aria-label="ZIP File Upload"
                      className="absolute inset-0 opacity-0 cursor-pointer"
                    />
                    <span className="text-[13px] text-dim font-medium">
                      {zipFile ? zipFile.name : 'Click to select or drag .zip here'}
                    </span>
                    {zipFile && (
                      <span className="text-[11px] text-ghost font-mono mt-1">
                        {(zipFile.size / 1024).toFixed(1)} KiB
                      </span>
                    )}
                  </div>
                </div>
              </div>
            )}

            {/* TAB CONTENT: AI generation */}
            {activeTab === 'ai' && (
              <div className="flex flex-col gap-4">
                {aiError && (
                  <div className="bg-red/10 border border-red/30 rounded-control p-3 text-red text-[13px] font-mono select-text" role="alert">
                    {aiError}
                  </div>
                )}

                <div className="flex flex-col gap-1.5 min-w-0">
                  <label htmlFor="ai-name-input" className="text-[10px] font-mono font-semibold tracking-[0.13em] uppercase text-ghost select-none">
                    Package Name · optional
                  </label>
                  <input
                    id="ai-name-input"
                    value={aiName}
                    onChange={(e) => setAiName(e.target.value)}
                    placeholder="e.g. generated-pkg (optional)"
                    className="bg-raise-2 border border-line-2 rounded-control text-fg text-[13px] px-3 py-2.5 outline-none w-full focus:border-faint transition-colors"
                  />
                </div>

                <div className="flex flex-col gap-1.5 min-w-0">
                  <label htmlFor="ai-desc-textarea" className="text-[10px] font-mono font-semibold tracking-[0.13em] uppercase text-ghost select-none">
                    AI Prompt / Description · what should the package do?
                  </label>
                  <textarea
                    id="ai-desc-textarea"
                    value={aiDescription}
                    onChange={(e) => setAiDescription(e.target.value)}
                    placeholder="e.g. A package that defines a department 'delivery' consuming event 'order' and producing 'shipment'."
                    className="bg-raise-2 border border-line-2 rounded-control text-fg text-[13px] px-3 py-2.5 outline-none w-full min-h-[100px] resize-y leading-normal focus:border-faint transition-colors"
                  />
                </div>

                <div className="flex items-center gap-2 select-none">
                  <Switch
                    checked={aiSave}
                    onCheckedChange={setAiSave}
                    aria-label="Save package to store automatically"
                  />
                  <span className="text-[13px] text-dim">
                    Save to store automatically (POST /api/v1/packages)
                  </span>
                </div>
              </div>
            )}

            {/* Honest Note */}
            <div className="font-mono text-[11px] text-ghost leading-relaxed bg-raise-2 border border-line rounded-control p-3 select-none">
              Stored in the hosted package store as <span className="text-faint">{`{name, files[], composed_deps[]}`}</span> — a duplicate name on create is a <b>409</b>; updates (PUT) and deletes (DELETE) are supported via the UI controls on the main screen. A package is loaded <b>once</b> at session start and composed into the one static graph — applying it = <b>stop the session, start a new one</b> (<span className="text-faint">POST /sessions/:id/stop</span> → poll <b>stopped</b> → <span className="text-faint">POST /sessions</span>). Create validates <b>structure only</b> (name <span className="text-faint">[A-Za-z0-9_-]+</span> · ≥1 file · ≤256 files · ≤1&nbsp;MiB/file · ≤12&nbsp;MiB total · must contain <span className="text-faint">departments/*/main.lua</span> or <span className="text-faint">raisers/*.lua</span>); <b>conformance runs at session start</b> — the session passes through <b>validating</b> and lands <b>failed</b> with the error on the session record if it doesn't conform. The source tree is <b>read-only at runtime</b>.
            </div>
          </div>
        </ModalSheet>
      </DialogContent>
    </Dialog>
  );
}

