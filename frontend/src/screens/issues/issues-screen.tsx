import { useState, useEffect } from 'react';
import { ConnectGitHub } from '@/components/auth/connect-github';
import { useGitHubAccounts } from '@/lib/hooks/useGitHubAccounts';
import {
  useGitHubIssues,
  useCreateIssue,
  usePatchIssue,
  useIssue,
  useComments,
  useCreateComment,
} from '@/lib/hooks/useGitHubIssues';
import { mapRepoTargetError } from '@/lib/api/goals';
import { cn } from '@/lib/utils';
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from '@/components/primitives/dialog';
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from '@/components/primitives/select';
import {
  Segmented,
  SegmentedList,
  SegmentedTrigger,
} from '@/components/primitives/segmented';

interface IssueDetailViewProps {
  owner: string;
  repo: string;
  number: number;
  account: string;
  onClose: () => void;
}

function IssueDetailView({ owner, repo, number, account, onClose }: IssueDetailViewProps) {
  const {
    data: issue,
    isLoading: issueLoading,
    isError: issueError,
    error: issueErrorObj,
  } = useIssue(owner, repo, number, account);
  const {
    data: comments,
    isLoading: commentsLoading,
    isError: commentsError,
  } = useComments(owner, repo, number, { account });

  const createCommentMutation = useCreateComment();
  const patchIssueMutation = usePatchIssue();

  const [commentText, setCommentText] = useState('');
  const [commentError, setCommentError] = useState<string | null>(null);
  const [stateError, setStateError] = useState<string | null>(null);

  const handleCreateComment = (e: React.FormEvent) => {
    e.preventDefault();
    if (!commentText.trim()) return;
    setCommentError(null);

    createCommentMutation.mutate(
      {
        owner,
        repo,
        number,
        body: commentText,
        account,
      },
      {
        onSuccess: () => {
          setCommentText('');
        },
        onError: (err) => {
          setCommentError(err instanceof Error ? err.message : 'Failed to post comment');
        },
      }
    );
  };

  const handleToggleState = () => {
    if (!issue) return;
    setStateError(null);
    const newState = issue.state === 'open' ? 'closed' : 'open';

    patchIssueMutation.mutate(
      {
        owner,
        repo,
        number,
        patch: {
          state: newState,
          account,
        },
      },
      {
        onError: (err) => {
          setStateError(err instanceof Error ? err.message : `Failed to ${newState} issue`);
        },
      }
    );
  };

  if (issueLoading) {
    return (
      <div className="flex items-center justify-center py-16 text-ghost font-mono text-[12px]">
        loading issue details...
      </div>
    );
  }

  if (issueError || !issue) {
    return (
      <div className="flex flex-col gap-4 py-8">
        <DialogTitle className="text-red">Failed to Load Issue</DialogTitle>
        <p className="text-dim text-[13px]">
          {issueErrorObj instanceof Error
            ? issueErrorObj.message
            : 'Unknown error loading issue details.'}
        </p>
        <button
          onClick={onClose}
          className="self-start font-ui text-[12.5px] border border-line hover:border-line-2 rounded-control px-4 py-2 mt-4 transition-colors cursor-pointer"
        >
          Close
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full overflow-hidden max-h-[80vh]">
      {/* Header */}
      <div className="flex items-start justify-between gap-4 pb-4 border-b border-line">
        <div className="flex flex-col gap-1 min-w-0">
          <span className="font-mono text-[11px] text-ghost truncate">
            {issue.repository} · #{issue.number} · fetched under @{account}
          </span>
          <DialogTitle className="text-[17px] font-semibold text-fg leading-snug">
            {issue.title}
          </DialogTitle>
        </div>
        <span
          className={cn(
            'text-[11px] font-mono uppercase tracking-wider px-2 py-0.5 rounded-chip border flex-shrink-0 mt-1',
            issue.state === 'open'
              ? 'bg-green/10 border-green/20 text-green'
              : 'bg-ghost/10 border-ghost/20 text-ghost'
          )}
        >
          {issue.state}
        </span>
      </div>

      {/* Body + Comments Scrollable */}
      <div className="flex-1 overflow-y-auto py-4 pr-1 flex flex-col gap-6 scrollbar-thin">
        {/* Description body */}
        <div className="flex flex-col gap-1.5">
          <span className="text-[10px] font-mono uppercase tracking-[0.13em] text-ghost">
            Description
          </span>
          {issue.body ? (
            <div className="bg-raise-2 border border-line rounded-control p-4 text-[13.5px] text-fg whitespace-pre-wrap font-sans leading-relaxed">
              {issue.body}
            </div>
          ) : (
            <div className="text-ghost text-[13.5px] italic py-2">No description provided.</div>
          )}
        </div>

        {/* State mutation errors */}
        {stateError && (
          <div className="bg-red/10 border border-red/25 rounded-control p-3 text-[12.5px] text-red">
            {stateError}
          </div>
        )}

        {/* Action buttons */}
        <div className="flex gap-2">
          <button
            onClick={handleToggleState}
            disabled={patchIssueMutation.isPending}
            className={cn(
              'font-ui font-semibold text-[12.5px] rounded-control px-3.5 py-1.5 transition-colors cursor-pointer border',
              issue.state === 'open'
                ? 'border-red text-red hover:bg-red/5'
                : 'bg-amber text-amber-ink border-transparent hover:brightness-[1.06]'
            )}
          >
            {patchIssueMutation.isPending
              ? 'Updating...'
              : issue.state === 'open'
              ? 'Close Issue'
              : 'Reopen Issue'}
          </button>
          <a
            href={issue.html_url}
            target="_blank"
            rel="noopener noreferrer"
            className="font-ui text-[12.5px] border border-line hover:border-line-2 hover:bg-raise rounded-control px-3.5 py-1.5 transition-colors cursor-pointer no-underline text-fg flex items-center justify-center"
          >
            View on GitHub ↗
          </a>
        </div>

        {/* Comments section */}
        <div className="flex flex-col gap-3 border-t border-line pt-4">
          <span className="text-[10px] font-mono uppercase tracking-[0.13em] text-ghost">
            Comments ({comments ? comments.length : '—'})
          </span>

          {commentsLoading && (
            <div className="text-[12px] font-mono text-ghost py-4">Loading comments...</div>
          )}

          {commentsError && (
            <div className="text-[12.5px] font-mono text-red py-4">Failed to load comments.</div>
          )}

          {!commentsLoading && !commentsError && (!comments || comments.length === 0) && (
            <div className="text-[13px] text-ghost italic py-4">No comments yet.</div>
          )}

          {comments && comments.length > 0 && (
            <div className="flex flex-col gap-3.5">
              {comments.map((comment) => (
                <div
                  key={comment.id}
                  className="border border-line rounded-control bg-raise-2 p-3 text-[12.5px] flex flex-col gap-1.5"
                >
                  <div className="flex items-center justify-between text-ghost text-[11px] font-mono border-b border-line/50 pb-1">
                    <span className="font-semibold text-fg">@{comment.user}</span>
                    <span>{new Date(comment.created_at).toLocaleString()}</span>
                  </div>
                  <p className="text-dim whitespace-pre-wrap leading-relaxed">{comment.body}</p>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* New Comment area */}
      <form onSubmit={handleCreateComment} className="border-t border-line pt-4 mt-auto">
        {commentError && (
          <div className="bg-red/10 border border-red/25 rounded-control p-2.5 text-[12.5px] text-red mb-3">
            {commentError}
          </div>
        )}
        <div className="flex flex-col gap-2">
          <textarea
            required
            placeholder="Add a comment..."
            value={commentText}
            onChange={(e) => setCommentText(e.target.value)}
            rows={3}
            className="rounded-control border border-line bg-raise py-2 px-3 text-[13px] text-fg placeholder:text-ghost focus-visible:outline-none resize-none font-sans"
          />
          <div className="flex justify-end gap-2">
            <button
              type="submit"
              disabled={createCommentMutation.isPending || !commentText.trim()}
              className="font-ui font-semibold text-[12.5px] bg-amber text-amber-ink hover:brightness-[1.06] rounded-control px-4 py-1.5 transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {createCommentMutation.isPending ? 'Commenting...' : 'Comment'}
            </button>
          </div>
        </div>
      </form>
    </div>
  );
}

export default function IssuesScreen() {
  const { data: accounts, isLoading: accountsLoading, isError: accountsError } = useGitHubAccounts();

  const [selectedAccount, setSelectedAccount] = useState<string>('all');
  const [stateFilter, setStateFilter] = useState<'open' | 'closed' | 'all'>('open');
  const [selectedIssue, setSelectedIssue] = useState<{
    owner: string;
    repo: string;
    number: number;
    account: string;
  } | null>(null);

  // Issue creation modal states
  const [isCreateModalOpen, setIsCreateModalOpen] = useState(false);
  const [createAccount, setCreateAccount] = useState<string>('');
  const [createRepo, setCreateRepo] = useState<string>('');
  const [createTitle, setCreateTitle] = useState<string>('');
  const [createBody, setCreateBody] = useState<string>('');
  const [createError, setCreateError] = useState<string | null>(null);

  const createIssueMutation = useCreateIssue();

  // Set default create account once accounts list is loaded
  useEffect(() => {
    const firstLogin = accounts?.[0]?.login;
    if (firstLogin && !createAccount) {
      setCreateAccount(firstLogin);
    }
  }, [accounts, createAccount]);

  const {
    data: issuesEnvelope,
    isLoading: issuesLoading,
    isError: issuesError,
  } = useGitHubIssues(
    accounts && accounts.length > 0
      ? {
          accounts:
            selectedAccount === 'all'
              ? accounts.map((a) => a.login).join(',')
              : selectedAccount,
          state: stateFilter,
        }
      : undefined
  );

  const handleCreateIssueSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    setCreateError(null);

    if (!createRepo.includes('/')) {
      setCreateError("Invalid repository format. Please use 'owner/repo'");
      return;
    }

    const [owner, repo] = createRepo.trim().split('/');
    if (!owner || !repo) {
      setCreateError("Invalid repository format. Please use 'owner/repo'");
      return;
    }

    createIssueMutation.mutate(
      {
        owner,
        repo,
        issue: {
          title: createTitle,
          body: createBody || undefined,
          account: createAccount || accounts?.[0]?.login,
        },
      },
      {
        onSuccess: () => {
          setIsCreateModalOpen(false);
          setCreateTitle('');
          setCreateBody('');
          setCreateRepo('');
        },
        onError: (err) => {
          setCreateError(mapRepoTargetError(err, 'issues'));
        },
      }
    );
  };

  if (accountsLoading) {
    return (
      <div className="p-6">
        <div className="flex items-center justify-center py-16 text-ghost font-mono text-[12px]">
          loading GitHub accounts...
        </div>
      </div>
    );
  }

  // If no accounts exist (or error), degrade to ConnectGitHub empty state
  if (accountsError || !accounts || accounts.length === 0) {
    return (
      <div className="p-6">
        <div className="flex flex-col items-center justify-center py-16 gap-3">
          <span className="text-ghost font-mono text-[12px]">
            {accountsError
              ? "GitHub status unknown — couldn't reach the connection service"
              : 'no GitHub accounts connected'}
          </span>
          <ConnectGitHub />
        </div>
      </div>
    );
  }

  return (
    <div className="p-6 flex flex-col gap-6">
      {/* Header section */}
      <div className="flex items-center justify-between gap-4 flex-wrap pb-3.5 border-b border-line">
        <div className="flex flex-col gap-1 min-w-0">
          <h1 className="font-display font-semibold text-[20px] text-fg leading-tight">
            GitHub Issues
          </h1>
          <span className="text-[12.5px] text-dim">
            view, create and manage issues across connected accounts
          </span>
        </div>
        <button
          onClick={() => {
            setIsCreateModalOpen(true);
            setCreateError(null);
            setCreateTitle('');
            setCreateBody('');
            setCreateRepo('');
          }}
          className="font-ui font-semibold text-[12.5px] bg-amber text-amber-ink hover:brightness-[1.06] rounded-control px-3.5 py-[7px] transition-colors cursor-pointer"
        >
          + New issue
        </button>
      </div>

      {/* Filters Toolbar */}
      <div className="flex items-center justify-between gap-4 flex-wrap pb-1 border-b border-line/50">
        <Segmented value={stateFilter} onValueChange={(v) => setStateFilter(v as 'open' | 'closed' | 'all')}>
          <SegmentedList>
            <SegmentedTrigger value="open">Open</SegmentedTrigger>
            <SegmentedTrigger value="closed">Closed</SegmentedTrigger>
            <SegmentedTrigger value="all">All</SegmentedTrigger>
          </SegmentedList>
        </Segmented>

        {accounts.length > 1 && (
          <div className="flex items-center gap-2">
            <span className="text-[12px] font-mono text-ghost">Account:</span>
            <Select value={selectedAccount} onValueChange={setSelectedAccount}>
              <SelectTrigger className="w-[180px]">
                <SelectValue placeholder="Select Account" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All Accounts</SelectItem>
                {accounts.map((acc) => (
                  <SelectItem key={acc.connection_id} value={acc.login}>
                    {acc.login}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}
      </div>

      {/* Content */}
      <div className="flex flex-col gap-8">
        {issuesLoading ? (
          <div className="flex items-center justify-center py-16 text-ghost font-mono text-[12px]">
            loading issues...
          </div>
        ) : issuesError ? (
          <div className="flex items-center justify-center py-16 text-ghost font-mono text-[12px]">
            Failed to load issues. Please check your connection.
          </div>
        ) : !issuesEnvelope ||
          !issuesEnvelope.results ||
          issuesEnvelope.results.length === 0 ? (
          <div className="flex items-center justify-center py-16 text-ghost font-mono text-[12px] border border-line rounded-card bg-raise">
            no issues found
          </div>
        ) : (
          issuesEnvelope.results.map((res) => {
            const hasIssues = res.issues && res.issues.length > 0;
            const rateLimit = res.rate_limit;
            const accountError = res.error;

            return (
              <div key={res.account} className="flex flex-col gap-3">
                <div className="flex items-baseline gap-3 pb-2 border-b border-line/60 flex-wrap">
                  <span className="font-mono text-[12.5px] font-semibold text-fg">
                    @{res.account}
                  </span>
                  {rateLimit && (
                    <span className="text-[11px] font-mono text-ghost">
                      (remaining rate limit: {rateLimit.remaining})
                    </span>
                  )}
                  {accountError && (
                    <span className="text-[11.5px] font-mono text-red bg-red/10 border border-red/20 rounded-chip px-2 py-0.5 ml-2">
                      Error: {accountError.message} ({accountError.kind})
                    </span>
                  )}
                </div>

                {accountError && !hasIssues && (
                  <div className="text-[13px] text-faint italic py-4">
                    Unable to load issues for this account.
                  </div>
                )}

                {!accountError && !hasIssues && (
                  <div className="text-[13px] text-ghost italic py-4">
                    No issues found for this account.
                  </div>
                )}

                {hasIssues && (
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    {res.issues.map((issue) => {
                      const parts = issue.repository.split('/');
                      const owner = parts[0] || issue.account;
                      const repo = parts[1] || issue.repository;

                      return (
                        <div
                          key={issue.id}
                          role="button"
                          tabIndex={0}
                          onClick={() =>
                            setSelectedIssue({
                              owner,
                              repo,
                              number: issue.number,
                              account: issue.account,
                            })
                          }
                          onKeyDown={(e) => {
                            if (e.key === 'Enter' || e.key === ' ') {
                              e.preventDefault();
                              setSelectedIssue({
                                owner,
                                repo,
                                number: issue.number,
                                account: issue.account,
                              });
                            }
                          }}
                          className="border border-line rounded-card bg-raise p-4 hover:border-line-2 hover:bg-raise-2 transition-all cursor-pointer flex flex-col justify-between gap-3"
                        >
                          <div className="flex flex-col gap-1 min-w-0">
                            <div className="flex items-center justify-between gap-2">
                              <span className="font-mono text-[11px] text-ghost truncate">
                                {issue.repository} · #{issue.number}
                              </span>
                              <span
                                className={cn(
                                  'text-[10px] font-mono uppercase tracking-wider px-1.5 py-0.5 rounded-chip border flex-shrink-0',
                                  issue.state === 'open'
                                    ? 'bg-green/10 border-green/20 text-green'
                                    : 'bg-ghost/10 border-ghost/20 text-ghost'
                                )}
                              >
                                {issue.state}
                              </span>
                            </div>
                            <h3 className="font-semibold text-fg text-[13.5px] leading-snug line-clamp-2">
                              {issue.title}
                            </h3>
                          </div>

                          <div className="flex items-center justify-between gap-2 pt-2 border-t border-line/30">
                            <div className="flex flex-wrap gap-1 max-w-[75%]">
                              {issue.labels.slice(0, 3).map((label) => (
                                <span
                                  key={label}
                                  className="text-[9.5px] font-mono px-1.5 py-0.5 rounded-control bg-raise-2 border border-line-2 text-dim truncate max-w-[80px]"
                                >
                                  {label}
                                </span>
                              ))}
                              {issue.labels.length > 3 && (
                                <span className="text-[9.5px] font-mono px-1.5 py-0.5 rounded-control bg-raise-2 border border-line-2 text-ghost">
                                  +{issue.labels.length - 3}
                                </span>
                              )}
                            </div>
                            {issue.comments > 0 && (
                              <span className="text-[11px] font-mono text-ghost flex items-center gap-1">
                                💬 {issue.comments}
                              </span>
                            )}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>

      {/* Create Issue Dialog */}
      <Dialog open={isCreateModalOpen} onOpenChange={setIsCreateModalOpen}>
        <DialogContent className="max-w-[480px]">
          <DialogTitle>Create New GitHub Issue</DialogTitle>
          <DialogDescription>
            Create an issue under one of your connected accounts.
          </DialogDescription>

          <form onSubmit={handleCreateIssueSubmit} className="flex flex-col gap-4 mt-4">
            {createError && (
              <div className="bg-red/10 border border-red/25 rounded-control p-3 text-[12.5px] text-red leading-relaxed whitespace-pre-wrap">
                {createError}
              </div>
            )}

            {accounts.length > 1 && (
              <div className="flex flex-col gap-1.5">
                <span className="text-[11px] font-mono text-ghost uppercase tracking-wide">
                  Account
                </span>
                <Select value={createAccount} onValueChange={setCreateAccount}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {accounts.map((acc) => (
                      <SelectItem key={acc.connection_id} value={acc.login}>
                        {acc.login}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}

            <div className="flex flex-col gap-1.5">
              <label htmlFor="create-repo-input" className="text-[11px] font-mono text-ghost uppercase tracking-wide">
                Repository (owner/repo)
              </label>
              <input
                id="create-repo-input"
                type="text"
                required
                placeholder="e.g. octocat/hello-world"
                value={createRepo}
                onChange={(e) => setCreateRepo(e.target.value)}
                className="rounded-control border border-line bg-raise py-2 px-3 text-[13px] text-fg placeholder:text-ghost focus-visible:outline-none focus:border-line-2 transition-colors"
              />
            </div>

            <div className="flex flex-col gap-1.5">
              <label htmlFor="create-title-input" className="text-[11px] font-mono text-ghost uppercase tracking-wide">
                Title
              </label>
              <input
                id="create-title-input"
                type="text"
                required
                placeholder="Issue title"
                value={createTitle}
                onChange={(e) => setCreateTitle(e.target.value)}
                className="rounded-control border border-line bg-raise py-2 px-3 text-[13px] text-fg placeholder:text-ghost focus-visible:outline-none focus:border-line-2 transition-colors"
              />
            </div>

            <div className="flex flex-col gap-1.5">
              <label htmlFor="create-body-textarea" className="text-[11px] font-mono text-ghost uppercase tracking-wide">
                Description (optional)
              </label>
              <textarea
                id="create-body-textarea"
                placeholder="Describe the issue..."
                value={createBody}
                onChange={(e) => setCreateBody(e.target.value)}
                rows={4}
                className="rounded-control border border-line bg-raise py-2 px-3 text-[13px] text-fg placeholder:text-ghost focus-visible:outline-none resize-none font-sans focus:border-line-2 transition-colors"
              />
            </div>

            <div className="flex justify-end gap-3 mt-2">
              <button
                type="button"
                onClick={() => setIsCreateModalOpen(false)}
                className="font-ui text-[12.5px] border border-line hover:border-line-2 hover:bg-raise rounded-control px-4 py-2 transition-colors cursor-pointer"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={createIssueMutation.isPending}
                className="font-ui font-semibold text-[12.5px] bg-amber text-amber-ink hover:brightness-[1.06] rounded-control px-4 py-2 transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {createIssueMutation.isPending ? 'Creating...' : 'Create'}
              </button>
            </div>
          </form>
        </DialogContent>
      </Dialog>

      {/* Issue Detail Dialog */}
      <Dialog
        open={!!selectedIssue}
        onOpenChange={(open) => {
          if (!open) setSelectedIssue(null);
        }}
      >
        <DialogContent className="max-w-[640px] max-h-[85vh] p-6 flex flex-col overflow-hidden">
          {selectedIssue && (
            <IssueDetailView
              owner={selectedIssue.owner}
              repo={selectedIssue.repo}
              number={selectedIssue.number}
              account={selectedIssue.account}
              onClose={() => setSelectedIssue(null)}
            />
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}
