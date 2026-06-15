export function ConfigError() {
  return (
    <div className="min-h-screen flex items-center justify-center bg-bg text-fg font-ui px-4 select-none">
      <div className="max-w-md w-full bg-raise border border-red/20 rounded-xl p-8 shadow-modal-seat relative overflow-hidden backdrop-blur-md">
        {/* Subtle decorative line at the top */}
        <div className="absolute top-0 left-0 right-0 h-1 bg-red" />
        
        <div className="flex items-center gap-3 mb-6">
          <div className="w-10 h-10 rounded-lg bg-red/10 border border-red/30 flex items-center justify-center flex-shrink-0 text-red font-mono text-xl font-bold">
            !
          </div>
          <div>
            <h1 className="font-display font-bold text-lg tracking-[0.01em] text-fg">
              Configuration Error
            </h1>
            <p className="text-xs text-ghost">
              ChronoAI fkst-hosted Auth
            </p>
          </div>
        </div>
        
        <div className="space-y-4">
          <p className="text-sm text-dim leading-relaxed">
            Authentication is required (<code className="text-xs font-mono bg-bg px-1 py-0.5 rounded text-amber">VITE_AUTH_REQUIRED=true</code>), but the NyxID identity configuration is missing or incomplete.
          </p>
          
          <div className="bg-bg/50 rounded-lg p-4 border border-line space-y-2.5 font-mono text-xs text-faint">
            <div className="flex justify-between items-center">
              <span>VITE_NYXID_BASE_URL</span>
              <span className="text-red font-semibold">Missing</span>
            </div>
            <div className="flex justify-between items-center">
              <span>VITE_NYXID_CLIENT_ID</span>
              <span className="text-red font-semibold">Missing</span>
            </div>
          </div>
          
          <div className="pt-2 text-xs text-ghost leading-relaxed">
            Please define these variables in your environment or <code className="bg-bg px-1 py-0.5 rounded text-fg">frontend/.env</code> file to enable authentication.
          </div>
        </div>
      </div>
    </div>
  );
}
