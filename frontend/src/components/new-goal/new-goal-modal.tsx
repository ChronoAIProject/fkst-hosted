import * as React from 'react';
import { useForm, Controller } from 'react-hook-form';
import { Link } from 'react-router-dom';
import { Dialog, DialogContent, DialogClose, DialogTrigger, DialogTitle } from '../primitives/dialog';
import { ModalSheet } from '../layout/modal-sheet';
import { Switch } from '../primitives/switch';
import { usePackagesList, usePackage } from '../../lib/hooks/usePackages';
import { useCreateGoal, useTriggerGoal } from '../../lib/hooks/useGoals';
import { mapRepoTargetError } from '../../lib/api/goals';
import { RepoRef, isApiErrorBody } from '../../lib/api/types';
import { ApiError } from '../../lib/api/client';

export interface NewGoalModalProps {
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  trigger?: React.ReactNode;
}

interface NewGoalFormValues {
  repository: string;
  title: string;
  description: string;
  package_names: string[];
  triggerOnCreate: boolean;
}

function PackageRow({
  name,
  selected,
  onToggle,
}: {
  name: string;
  selected: boolean;
  onToggle: () => void;
}) {
  const { data: pkg, isLoading, isError } = usePackage(name);

  const content = (() => {
    if (isLoading) {
      return (
        <div className="font-mono text-[10px] text-ghost ml-2">Loading details...</div>
      );
    }

    if (isError || !pkg) {
      return (
        <span className="font-mono text-[10px] text-red ml-2" data-testid="package-row-unknown">
          (details unknown)
        </span>
      );
    }

    return (
      <div className="flex items-center gap-1.5 ml-auto">
        {pkg.composed_deps && pkg.composed_deps.length > 0 ? (
          pkg.composed_deps.map((dep) => (
            <span
              key={dep}
              className="px-2 py-0.5 rounded bg-raise text-ghost text-[10px] border border-line font-mono"
            >
              dep · {dep}
            </span>
          ))
        ) : (
          <span className="text-[10px] text-ghost font-mono">no deps</span>
        )}
      </div>
    );
  })();

  return (
    <label
      className={`flex items-center gap-2.5 px-3 py-[9px] text-[12.5px] border-t border-line first:border-t-0 hover:bg-raise cursor-pointer transition-colors ${
        selected ? 'bg-raise text-fg font-medium' : 'bg-raise-2 text-dim'
      }`}
      data-testid={`package-row-${name}`}
    >
      <input
        type="checkbox"
        checked={selected}
        onChange={onToggle}
        className="rounded border-line-2 bg-raise-2 text-amber focus:ring-amber focus:ring-offset-0 cursor-pointer h-3.5 w-3.5"
        data-testid={`package-checkbox-${name}`}
      />
      <span className="font-sans">{name}</span>
      {content}
    </label>
  );
}

interface PackageGraphSectionProps {
  selectedPackages: string[];
  onChange: (names: string[]) => void;
  error?: string;
}

function PackageGraphSection({ selectedPackages, onChange, error }: PackageGraphSectionProps) {
  const { data: packages, isLoading, isError } = usePackagesList();

  const handleToggle = (name: string) => {
    if (selectedPackages.includes(name)) {
      onChange(selectedPackages.filter((n) => n !== name));
    } else {
      onChange([...selectedPackages, name]);
    }
  };

  return (
    <div className="flex flex-col gap-1.5">
      <span className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost">
        Deployment packages · multi-select
      </span>
      
      {isLoading && (
        <div className="flex flex-col border border-line rounded-[9px] overflow-hidden" data-testid="package-graph-loading">
          <div className="flex items-center justify-between gap-2.5 px-3 py-[9px] text-[12.5px] text-dim bg-raise-2 border-t border-line first:border-t-0">
            <span className="font-mono text-xs text-ghost">Loading package graph...</span>
          </div>
        </div>
      )}

      {isError && (
        <div className="flex flex-col border border-line rounded-[9px] overflow-hidden" data-testid="package-graph-unknown">
          <div className="flex items-center justify-between gap-2.5 px-3 py-[9px] text-[12.5px] text-dim bg-raise-2 border-t border-line first:border-t-0">
            <span className="text-red font-mono text-xs">unknown (unreachable)</span>
          </div>
        </div>
      )}

      {!isLoading && !isError && packages && packages.length === 0 && (
        <div className="flex flex-col border border-line rounded-[9px] overflow-hidden" data-testid="package-graph-empty">
          <div className="flex items-center justify-center gap-2.5 px-3 py-[9px] text-[12.5px] text-dim bg-raise-2 border-t border-line first:border-t-0">
            <span className="font-mono text-xs text-ghost">no packages in the hosted store</span>
          </div>
        </div>
      )}

      {!isLoading && !isError && packages && packages.length > 0 && (
        <div className="flex flex-col border border-line rounded-[9px] overflow-hidden" data-testid="package-graph-list">
          {packages.map((name) => (
            <PackageRow
              key={name}
              name={name}
              selected={selectedPackages.includes(name)}
              onToggle={() => handleToggle(name)}
            />
          ))}
        </div>
      )}

      {error && (
        <span className="text-red font-mono text-[10.5px]" data-testid="package-selection-error">
          {error}
        </span>
      )}

      <div className="font-mono text-[10.5px] text-ghost mt-[7px] leading-[1.55]">
        deployment-wide, applies on restart — not per-goal · <Link to="/packages" className="text-faint underline underline-offset-2">View packages →</Link>
      </div>
    </div>
  );
}

export function NewGoalModal({ open, onOpenChange, trigger }: NewGoalModalProps) {
  const [submitError, setSubmitError] = React.useState<string | null>(null);

  const { control, register, handleSubmit, reset, formState: { errors } } = useForm<NewGoalFormValues>({
    defaultValues: {
      repository: '',
      title: '',
      description: '',
      package_names: [],
      triggerOnCreate: false,
    },
  });

  const { mutateAsync: createGoal, isPending: isCreating } = useCreateGoal();
  const { mutateAsync: triggerGoal, isPending: isTriggering } = useTriggerGoal();
  const isSubmitting = isCreating || isTriggering;

  React.useEffect(() => {
    if (!open) {
      reset({
        repository: '',
        title: '',
        description: '',
        package_names: [],
        triggerOnCreate: false,
      });
      setSubmitError(null);
    }
  }, [open, reset]);

  const onSubmit = async (values: NewGoalFormValues) => {
    setSubmitError(null);
    let createdGoal;
    try {
      let repo: RepoRef | null = null;
      if (values.repository.trim() !== '') {
        const parts = values.repository.trim().split('/');
        repo = {
          owner: (parts[0] || '').trim(),
          name: (parts[1] || '').trim(),
        };
      }

      // 1. Create the goal
      createdGoal = await createGoal({
        title: values.title.trim(),
        description: values.description.trim(),
        package_names: values.package_names,
        repo,
      });
    } catch (err) {
      console.error('Failed to create goal:', err);
      if (err instanceof ApiError) {
        let message = '';
        if (err.body) {
          if (isApiErrorBody(err.body)) {
            message = err.body.message;
          } else if (typeof err.body === 'object' && 'message' in err.body) {
            const bodyRecord = err.body as Record<string, unknown>;
            if (typeof bodyRecord.message === 'string') {
              message = bodyRecord.message;
            }
          }
        }
        if (!message) {
          message = err.message;
        }
        setSubmitError(message || 'An unexpected error occurred');
      } else {
        setSubmitError(err instanceof Error ? err.message : 'An unexpected error occurred');
      }
      return;
    }

    // 2. Optional: Trigger goal run immediately
    if (values.triggerOnCreate) {
      try {
        await triggerGoal({
          id: createdGoal.id,
          req: {
            repo: createdGoal.repo,
            repo_mode: 'existing',
          },
        });
      } catch (err) {
        console.error('Failed to trigger goal:', err);
        setSubmitError(mapRepoTargetError(err, 'trigger'));
        return;
      }
    }

    onOpenChange?.(false);
    reset();
  };

  const modalContent = (
    <DialogContent
      showClose={false}
      className="p-0 border-0 bg-transparent w-full max-w-[560px]"
      style={{ boxShadow: 'none' }}
      aria-describedby={undefined}
      data-testid="new-goal-modal-content"
    >
      <DialogTitle className="sr-only">New goal</DialogTitle>
      <form onSubmit={handleSubmit(onSubmit)}>
        <ModalSheet
          title={<span>New goal</span>}
          meta="creates a hosted goal record — via NyxID"
          className="relative max-h-[88vh]"
          closeButtonSlot={
            <DialogClose asChild>
              <button
                type="button"
                className="absolute top-[14px] right-[14px] w-[30px] h-[30px] rounded-control border border-line bg-raise-2 text-faint hover:text-fg hover:border-faint flex items-center justify-center cursor-pointer transition-colors focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2"
                aria-label="Close"
              >
                <span className="text-[17px] leading-none" aria-hidden="true">✕</span>
              </button>
            </DialogClose>
          }
          actionLeftSlot={
            <span className="text-ghost font-mono text-[11.5px]" data-testid="submit-note">
              requires NyxID sign-in
            </span>
          }
          actionButtonsSlot={
            <>
              <DialogClose asChild>
                <button
                  type="button"
                  className="px-3 py-1.5 bg-raise border border-line-2 text-dim hover:text-fg hover:border-faint rounded-control text-xs font-medium transition-colors cursor-pointer outline-none focus-visible:outline-2 focus-visible:outline-amber"
                >
                  Cancel
                </button>
              </DialogClose>
              <button
                type="submit"
                className="font-sans font-semibold text-[12.5px] text-amber-ink bg-amber border-0 rounded-[9px] px-[15px] py-2 cursor-pointer transition-all hover:not-disabled:brightness-[1.06] disabled:cursor-not-allowed disabled:opacity-[0.62]"
                disabled={isSubmitting}
                data-testid="submit-button"
              >
                {isSubmitting ? 'Creating...' : 'Create goal'}
              </button>
            </>
          }
        >
          <div className="flex flex-col gap-3.5 pt-1.5 pb-2">
            {/* Repository Input */}
            <div className="flex flex-col gap-1.5">
              <label htmlFor="repo-field" className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost">
                Repository (optional)
              </label>
              <input
                id="repo-field"
                {...register('repository', {
                  validate: (val) => {
                    if (!val || val.trim() === '') return true;
                    const parts = val.trim().split('/');
                    if (parts.length !== 2 || !parts[0] || !parts[1]) {
                      return "Repository must be in the format 'owner/repo'";
                    }
                    return true;
                  }
                })}
                className="bg-raise-2 border border-line-2 rounded-[9px] text-fg font-sans text-[13px] px-3 py-[9px] outline-none w-full transition-colors duration-120 ease focus:border-faint focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2"
                placeholder="owner/repo"
                data-testid="repo-input"
              />
              {errors.repository && (
                <span className="text-red font-mono text-[10.5px]" data-testid="repo-validation-error">
                  {errors.repository.message}
                </span>
              )}
            </div>

            {/* Title Input */}
            <div className="flex flex-col gap-1.5">
              <label htmlFor="title-field" className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost">
                Title
              </label>
              <input
                id="title-field"
                {...register('title', {
                  required: 'Title is required',
                  validate: (v) => v.trim() !== '' || 'Title is required',
                })}
                className="bg-raise-2 border border-line-2 rounded-[9px] text-fg font-sans text-[13px] px-3 py-[9px] outline-none w-full transition-colors duration-120 ease focus:border-faint focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2"
                placeholder="e.g. Add caching layer to database queries"
                data-testid="title-input"
              />
              {errors.title && (
                <span className="text-red font-mono text-[10.5px]" data-testid="title-validation-error">
                  {errors.title.message}
                </span>
              )}
            </div>

            {/* Description Textarea */}
            <div className="flex flex-col gap-1.5">
              <label htmlFor="desc-field" className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost">
                Description
              </label>
              <textarea
                id="desc-field"
                {...register('description', {
                  required: 'Description is required',
                  validate: (v) => v.trim() !== '' || 'Description is required',
                })}
                className="bg-raise-2 border border-line-2 rounded-[9px] text-fg font-sans text-[13px] px-3 py-[9px] outline-none w-full transition-colors duration-120 ease focus:border-faint focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2 min-h-[84px] resize-y leading-[1.5]"
                placeholder="What should change, and why. The engine reads the title, description, and packages at intake."
                data-testid="description-textarea"
              />
              {errors.description && (
                <span className="text-red font-mono text-[10.5px]" data-testid="description-validation-error">
                  {errors.description.message}
                </span>
              )}
            </div>

            {/* Packages Section */}
            <Controller
              name="package_names"
              control={control}
              rules={{
                validate: (value) =>
                  (value && value.length > 0) || 'At least one package must be selected',
              }}
              render={({ field, fieldState }) => (
                <PackageGraphSection
                  selectedPackages={field.value}
                  onChange={field.onChange}
                  error={fieldState.error?.message}
                />
              )}
            />

            {/* Trigger On Create Toggle */}
            <div className="flex items-center justify-between gap-3 bg-raise-2 border border-line rounded-[9px] px-3.5 py-2.5">
              <div className="flex flex-col gap-0.5">
                <span className="font-sans font-medium text-[12.5px] text-fg">Trigger immediately</span>
                <span className="font-mono text-[10px] text-ghost">Spawn a new agent session right after creation</span>
              </div>
              <Controller
                name="triggerOnCreate"
                control={control}
                render={({ field }) => (
                  <Switch
                    checked={field.value}
                    onCheckedChange={field.onChange}
                    aria-label="Trigger immediately"
                    data-testid="trigger-on-create-switch"
                  />
                )}
              />
            </div>

            {submitError && (
              <div className="text-red font-mono text-xs bg-raise-2 border border-red/20 rounded-[9px] px-3.5 py-2.5" data-testid="submit-error">
                {submitError}
              </div>
            )}

            {/* Mockup's honest explanation copy */}
            <div className="font-mono text-[11px] text-ghost leading-[1.6] bg-raise-2 border border-line rounded-[9px] px-[13px] py-2.5">
              Creates a hosted goal record. The engine's next <b className="text-faint font-medium">~5-min poll</b> raises it → it enters the development cycle. Deployment packages apply wide on restart.
            </div>

            {/* The engine-pickup footnote */}
            <div className="font-mono text-[10.5px] text-ghost mt-[7px] leading-[1.55]" data-testid="engine-pickup-footnote">
              the engine's next ~5-min poll picks it up → development cycle
            </div>
          </div>
        </ModalSheet>
      </form>
    </DialogContent>
  );

  if (trigger) {
    return (
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogTrigger asChild>{trigger}</DialogTrigger>
        {modalContent}
      </Dialog>
    );
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      {modalContent}
    </Dialog>
  );
}
