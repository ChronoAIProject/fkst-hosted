
export interface ConnectGitHubProps {
  className?: string;
}

export function ConnectGitHub({ className }: ConnectGitHubProps) {
  const connectUrl = import.meta.env.VITE_NYXID_CONNECT_GITHUB_URL;

  if (!connectUrl) {
    return (
      <div className="flex flex-col items-center gap-1.5">
        <button
          disabled
          type="button"
          className="font-ui font-semibold text-[12.5px] rounded-control px-3.5 py-[7px] bg-amber/50 text-amber-ink/50 cursor-not-allowed opacity-50 select-none"
        >
          Connect GitHub
        </button>
        <span className="text-[11px] text-ghost font-mono">
          GitHub connection URL is not configured
        </span>
      </div>
    );
  }

  return (
    <a
      href={connectUrl}
      className={`font-ui font-semibold text-[12.5px] rounded-control px-3.5 py-[7px] bg-amber text-amber-ink hover:brightness-[1.06] cursor-pointer transition-colors no-underline inline-block text-center ${className || ''}`}
    >
      Connect GitHub
    </a>
  );
}
