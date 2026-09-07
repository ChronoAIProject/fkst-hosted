#!/usr/bin/env node
// Artifact renderer (spec section 28.1's "artifact renderer" role).
//
// Turns the journey's raw output into the two rendered binaries:
//
//   * the slide deck PDF, rendered from the committed Marp source; and
//   * the demo MP4, built from the journey's own video recording, with a title
//     card taken from the deck's first slide and a caption track derived from
//     the journey's checkpoints.
//
// WHY the title card is the deck's own first slide rather than a separately
// designed frame: section 23.8 forbids an artifact from reusing a claim or image
// simply because it appeared elsewhere, and the cheapest way to honour that is to
// have exactly one definition of the title and derive both artifacts from it.
//
// WHY captions come from journey checkpoints rather than a written script: a
// caption authored away from the step it describes is the first thing to go stale
// when the journey changes. Here they cannot diverge — the journey emits them.

import { execFile, spawn } from 'node:child_process';
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { log } from './log.ts';

const execFileAsync = promisify(execFile);

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, '..', '..', '..');
const OUT = join(REPO, 'tools', 'evolution', 'out');
const SLIDES_SRC = join(REPO, '.fkst', 'evolution', 'slides', 'product-intro.md');
// The rendered PDF is a Release asset, NOT a committed file: section 12.2
// forbids "large videos, PDFs, or presentation binaries" under the Evolution
// root, and section 24.1 sends exactly those formats to GitHub Releases. Only
// the editable Marp source is committed.
const SLIDES_PDF = join(OUT, 'product-intro.pdf');
const CHECKPOINTS = join(OUT, 'checkpoints.json');

/** Seconds the title card is held before the recording begins. */
const TITLE_SECONDS = 2.5;
/** The hold `checkpoint()` applies after every capture — see the journey. */
const FINAL_HOLD_MS = 1200;
/** Output geometry, pinned to the journey's viewport (section 23.6). */
const WIDTH = 1440;
const HEIGHT = 900;
const FPS = 25;

interface Checkpoint {
  id: string;
  offsetMs: number;
  caption: string;
}

/**
 * Chromium for Marp.
 *
 * Marp drives a browser to render. Rather than let it download its own — a
 * change to machine state outside this project — point it at the Chromium
 * Playwright already installed for the journeys, so the deck and the demo are
 * rendered by the same engine at the same revision.
 */
async function chromePath(): Promise<string> {
  if (process.env.CHROME_PATH) return process.env.CHROME_PATH;
  // Imported directly rather than shelled out to: `playwright-core` resolves
  // from the repository root by ordinary upward lookup, and spawning a helper
  // node process to ask the same question just adds a way for the render to
  // hang with no output.
  const { chromium } = await import('playwright-core');
  return chromium.executablePath();
}

/**
 * Run a renderer, with stdin CLOSED.
 *
 * `stdio[0]: 'ignore'` is load-bearing, not tidiness. Given an open stdin pipe
 * that never closes, marp-cli waits for markdown on it and the render hangs
 * forever with no output — which looks exactly like a slow PDF conversion.
 */
function run(cmd: string, args: string[], env: NodeJS.ProcessEnv = {}): Promise<void> {
  log.debug('exec', { cmd, args: args.join(' ') });
  return new Promise((resolveRun, rejectRun) => {
    const child = spawn(cmd, args, {
      cwd: REPO,
      env: { ...process.env, ...env },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    const err: Buffer[] = [];
    child.stdout.on('data', (c: Buffer) => log.debug(cmd, { out: c.toString('utf8').trim() }));
    child.stderr.on('data', (c: Buffer) => err.push(c));
    child.on('error', rejectRun);
    child.on('close', (code) => {
      const text = Buffer.concat(err).toString('utf8').trim();
      if (code === 0) {
        if (text) log.debug(`${cmd} stderr`, { text: text.slice(0, 400) });
        return resolveRun();
      }
      rejectRun(new Error(`${cmd} exited ${code}: ${text.slice(-2000)}`));
    });
  });
}

/** Locate the journey's video recording under the Playwright output directory. */
async function findJourneyVideo(): Promise<string> {
  const root = join(OUT, 'journeys');
  const entries = await readdir(root, { withFileTypes: true, recursive: true });
  const video = entries.find((e) => e.isFile() && e.name.endsWith('.webm'));
  if (!video) {
    throw new Error(`no journey recording under ${root} — run \`npm run evolution:journeys\` first`);
  }
  return join(video.parentPath ?? root, video.name);
}

/** Duration of a media file in milliseconds, via ffprobe. */
async function durationMs(path: string): Promise<number> {
  const { stdout } = await execFileAsync(
    'ffprobe',
    ['-v', 'error', '-show_entries', 'format=duration', '-of', 'csv=p=0', path],
    { encoding: 'utf8' }
  );
  return Math.round(Number(stdout.trim()) * 1000);
}

function vttTime(ms: number): string {
  const clamped = Math.max(0, ms);
  const h = String(Math.floor(clamped / 3_600_000)).padStart(2, '0');
  const m = String(Math.floor((clamped % 3_600_000) / 60_000)).padStart(2, '0');
  const s = String(Math.floor((clamped % 60_000) / 1000)).padStart(2, '0');
  const milli = String(clamped % 1000).padStart(3, '0');
  return `${h}:${m}:${s}.${milli}`;
}

/**
 * Build the caption track.
 *
 * `leadMs` corrects for the gap between Playwright starting the recording (at
 * browser-context creation) and the journey's first line running. It is measured
 * rather than assumed: the journey's final checkpoint is followed by a known
 * hold, so the leftover duration is the lead. It is clamped because a teardown
 * that ran long would otherwise push every caption late.
 */
export function buildVtt(checkpoints: Checkpoint[], leadMs: number, offsetMs: number): string {
  const lines = ['WEBVTT', ''];
  checkpoints.forEach((cp, i) => {
    const start = cp.offsetMs + leadMs + offsetMs;
    const next = checkpoints[i + 1];
    const end = next ? next.offsetMs + leadMs + offsetMs : start + 3000;
    lines.push(`${cp.id}`, `${vttTime(start)} --> ${vttTime(end)}`, cp.caption, '');
  });
  return `${lines.join('\n')}\n`;
}

/** Render the committed Marp source to PDF and to a title-card PNG. */
async function renderDeck(): Promise<string> {
  const chrome = await chromePath();
  log.info('rendering deck', { source: SLIDES_SRC, chrome });
  const marp = join(REPO, 'tools', 'evolution', 'node_modules', '.bin', 'marp');
  // `--allow-local-files` is required because the deck embeds the journey's
  // screenshots from the managed screenshots subtree by relative path.
  await run(marp, [SLIDES_SRC, '--pdf', '--allow-local-files', '-o', SLIDES_PDF], { CHROME_PATH: chrome });

  const titleCard = join(OUT, 'title-card.png');
  await run(marp, [SLIDES_SRC, '--image', 'png', '--allow-local-files', '-o', titleCard], {
    CHROME_PATH: chrome,
  });
  return titleCard;
}

/** Compose the demo MP4: title card, journey recording, embedded captions. */
async function renderVideo(titleCard: string): Promise<string> {
  const source = await findJourneyVideo();
  const checkpoints = JSON.parse(await readFile(CHECKPOINTS, 'utf8')) as Checkpoint[];
  const recordedMs = await durationMs(source);
  const lastOffset = checkpoints[checkpoints.length - 1]?.offsetMs ?? 0;
  const leadMs = Math.min(Math.max(recordedMs - (lastOffset + FINAL_HOLD_MS), 0), 1500);
  log.info('composing video', { source, recordedMs, leadMs });

  const vtt = buildVtt(checkpoints, leadMs, TITLE_SECONDS * 1000);
  const vttPath = join(OUT, 'captions.vtt');
  await writeFile(vttPath, vtt, 'utf8');

  const silent = join(OUT, 'demo-silent.mp4');
  await run('ffmpeg', [
    '-y',
    '-loop', '1', '-t', String(TITLE_SECONDS), '-i', titleCard,
    '-i', source,
    '-filter_complex',
    `[0:v]scale=${WIDTH}:${HEIGHT}:force_original_aspect_ratio=decrease,` +
      `pad=${WIDTH}:${HEIGHT}:(ow-iw)/2:(oh-ih)/2:color=0x0a0a0f,setsar=1,fps=${FPS},format=yuv420p[a];` +
      `[1:v]scale=${WIDTH}:${HEIGHT}:force_original_aspect_ratio=decrease,` +
      `pad=${WIDTH}:${HEIGHT}:(ow-iw)/2:(oh-ih)/2:color=0x0a0a0f,setsar=1,fps=${FPS},format=yuv420p[b];` +
      `[a][b]concat=n=2:v=1:a=0[v]`,
    '-map', '[v]',
    '-c:v', 'libx264', '-preset', 'medium', '-crf', '23',
    '-pix_fmt', 'yuv420p', '-movflags', '+faststart',
    silent,
  ]);

  // Captions are muxed in a second pass rather than in the filter graph: burning
  // them in would make them impossible to turn off, and section 23.9 wants a
  // localized track to be substitutable without re-encoding the picture.
  const final = join(OUT, 'queue-work-item.mp4');
  await run('ffmpeg', [
    '-y', '-i', silent, '-i', vttPath,
    '-map', '0:v', '-map', '1:s',
    '-c:v', 'copy', '-c:s', 'mov_text',
    '-metadata:s:s:0', 'language=eng',
    '-movflags', '+faststart',
    final,
  ]);
  return final;
}

async function main(): Promise<void> {
  await mkdir(OUT, { recursive: true });
  const titleCard = await renderDeck();
  const video = await renderVideo(titleCard);
  log.info('render complete', { deck: SLIDES_PDF, video });
  process.stdout.write(`${JSON.stringify({ deck: SLIDES_PDF, video }, null, 2)}\n`);
}

main().catch((error) => {
  log.error('render failed', { error: error instanceof Error ? error.message : String(error) });
  process.exitCode = 1;
});
