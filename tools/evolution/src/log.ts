// Structured, level-filtered logging for the Evolution toolchain.
//
// WHY stderr: every CLI command writes machine-readable JSON or a fingerprint to
// stdout, and callers pipe it. Diagnostics that shared stdout would corrupt that
// contract, so all logging goes to stderr regardless of level.

export type Level = 'debug' | 'info' | 'warn' | 'error';

const ORDER: Record<Level, number> = { debug: 10, info: 20, warn: 30, error: 40 };

function configuredLevel(): Level {
  const raw = (process.env.FKST_EVOLUTION_LOG_LEVEL ?? 'info').toLowerCase();
  return raw in ORDER ? (raw as Level) : 'info';
}

let threshold = ORDER[configuredLevel()];

/** Re-read the threshold — used by the CLI after it parses a `--log-level` flag. */
export function setLevel(level: Level): void {
  threshold = ORDER[level];
}

function emit(level: Level, message: string, context?: Record<string, unknown>): void {
  if (ORDER[level] < threshold) return;
  const suffix = context
    ? ' ' +
      Object.entries(context)
        .map(([k, v]) => `${k}=${typeof v === 'string' ? v : JSON.stringify(v)}`)
        .join(' ')
    : '';
  process.stderr.write(`[evolution] ${level.padEnd(5)} ${message}${suffix}\n`);
}

export const log = {
  debug: (m: string, c?: Record<string, unknown>) => emit('debug', m, c),
  info: (m: string, c?: Record<string, unknown>) => emit('info', m, c),
  warn: (m: string, c?: Record<string, unknown>) => emit('warn', m, c),
  error: (m: string, c?: Record<string, unknown>) => emit('error', m, c),
};
