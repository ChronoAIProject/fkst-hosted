import * as React from 'react';
import { useForm, Controller } from 'react-hook-form';
import { Link } from 'react-router-dom';
import { Dialog, DialogContent, DialogClose, DialogTrigger, DialogTitle } from '../primitives/dialog';
import { ModalSheet } from '../layout/modal-sheet';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../primitives/select';
import { usePackagesList, usePackage } from '../../lib/hooks/usePackages';

export interface NewGoalModalProps {
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  trigger?: React.ReactNode;
}

interface NewGoalFormValues {
  repository: string;
  title: string;
  description: string;
}

function PackageRow({ name }: { name: string }) {
  const { data: pkg, isLoading, isError } = usePackage(name);

  if (isLoading) {
    return (
      <div className="flex items-center justify-between gap-2.5 px-3 py-[9px] text-[12.5px] text-dim bg-raise-2 border-t border-line first:border-t-0" data-testid={`package-row-${name}`}>
        <span>{name}</span>
        <span className="font-mono text-[10px] text-ghost ml-2">Loading details...</span>
      </div>
    );
  }

  if (isError || !pkg) {
    return (
      <div className="flex items-center justify-between gap-2.5 px-3 py-[9px] text-[12.5px] text-dim bg-raise-2 border-t border-line first:border-t-0" data-testid={`package-row-${name}`}>
        <span>{name}</span>
        <span className="font-mono text-[10px] text-red ml-2" data-testid="package-row-unknown">
          (details unknown)
        </span>
      </div>
    );
  }

  return (
    <div className="flex items-center justify-between gap-2.5 px-3 py-[9px] text-[12.5px] text-dim bg-raise-2 border-t border-line first:border-t-0" data-testid={`package-row-${name}`}>
      <span>{pkg.name}</span>
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
    </div>
  );
}

function PackageGraphSection() {
  const { data: packages, isLoading, isError } = usePackagesList();

  return (
    <div className="flex flex-col gap-1.5">
      <span className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost">Deployment package graph · read-only</span>
      
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
            <PackageRow key={name} name={name} />
          ))}
        </div>
      )}

      <div className="font-mono text-[10.5px] text-ghost mt-[7px] leading-[1.55]">
        deployment-wide, applies on restart — not per-goal · <Link to="/packages" className="text-faint underline underline-offset-2">View packages →</Link>
      </div>
    </div>
  );
}

export function NewGoalModal({ open, onOpenChange, trigger }: NewGoalModalProps) {
  const { control, register, handleSubmit, reset } = useForm<NewGoalFormValues>({
    defaultValues: {
      repository: '',
      title: '',
      description: '',
    },
  });

  React.useEffect(() => {
    if (!open) {
      reset({
        repository: '',
        title: '',
        description: '',
      });
    }
  }, [open, reset]);

  const onSubmit = () => {
    // Submission is disabled in v1
  };

  const repoOptions = [
    { value: 'example-org/repo-a', label: 'example-org/repo-a — example' },
    { value: 'example-org/repo-b', label: 'example-org/repo-b — example' },
    { value: 'example-org/repo-c', label: 'example-org/repo-c — example' },
  ];

  // DialogContent supplies behavior; ModalSheet supplies the visual sheet+shadow — suppressing the primitive's seat-shadow to avoid double shadows. Wave-1 gap: DialogContent should expose a 'bare' variant.
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
          meta="creates a GitHub issue + fkst-dev:enabled — via NyxID"
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
                className="font-sans font-semibold text-[12.5px] text-amber-ink bg-amber border-0 rounded-[9px] px-[15px] py-2 cursor-pointer transition-all duration-120 ease hover:not-disabled:brightness-[1.06] disabled:cursor-not-allowed disabled:opacity-[0.62]"
                disabled
                data-testid="submit-button"
              >
                Create issue &amp; enable
              </button>
            </>
          }
        >
          <div className="flex flex-col gap-3.5 pt-1.5 pb-2">
            {/* Repository Select */}
            <div className="flex flex-col gap-1.5">
              <span id="repo-label" className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost">
                Repository
              </span>
              <Controller
                name="repository"
                control={control}
                render={({ field }) => (
                  <Select value={field.value} onValueChange={field.onChange}>
                    <SelectTrigger
                      aria-labelledby="repo-label"
                      aria-label="Repository"
                      className="w-full bg-raise-2 border-line-2 text-fg h-[38px] py-2 px-3 rounded-[9px] focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2"
                      data-testid="repo-select-trigger"
                    >
                      <SelectValue placeholder="Select repository..." />
                    </SelectTrigger>
                    <SelectContent>
                      {repoOptions.map((opt) => (
                        <SelectItem key={opt.value} value={opt.value}>
                          {opt.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                )}
              />
            </div>

            {/* Title Input */}
            <div className="flex flex-col gap-1.5">
              <label htmlFor="title-field" className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost">
                Title
              </label>
              <input
                id="title-field"
                {...register('title')}
                className="bg-raise-2 border border-line-2 rounded-[9px] text-fg font-sans text-[13px] px-3 py-[9px] outline-none w-full transition-colors duration-120 ease focus:border-faint focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2"
                placeholder="e.g. Add caching layer to database queries"
                data-testid="title-input"
              />
            </div>

            {/* Description Textarea */}
            <div className="flex flex-col gap-1.5">
              <label htmlFor="desc-field" className="font-mono font-semibold text-[10px] tracking-[0.13em] uppercase text-ghost">
                Description
              </label>
              <textarea
                id="desc-field"
                {...register('description')}
                className="bg-raise-2 border border-line-2 rounded-[9px] text-fg font-sans text-[13px] px-3 py-[9px] outline-none w-full transition-colors duration-120 ease focus:border-faint focus-visible:outline-2 focus-visible:outline-amber focus-visible:outline-offset-2 min-h-[84px] resize-y leading-[1.5]"
                placeholder="What should change, and why. The engine reads the full issue + comments at intake."
                data-testid="description-textarea"
              />
            </div>

            {/* Packages Section */}
            <PackageGraphSection />

            {/* Mockup's honest explanation copy */}
            <div className="font-mono text-[11px] text-ghost leading-[1.6] bg-raise-2 border border-line rounded-[9px] px-[13px] py-2.5">
              Creates a GitHub issue labeled <b className="text-faint font-medium">fkst-dev:enabled</b> via NyxID. The engine's next <b className="text-faint font-medium">~5-min poll</b> raises it → <b className="text-faint font-medium">intake_judge</b> decides if it's auto-developable → it enters the <b className="text-faint font-medium">Design</b> stage. A real GitHub write on your account (via NyxID) — not an engine write, so it works in DRY-RUN too.
            </div>

            {/* The engine-pickup footnote */}
            <div className="font-mono text-[10.5px] text-ghost mt-[7px] leading-[1.55]" data-testid="engine-pickup-footnote">
              the engine's next ~5-min poll picks it up → Design stage
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
