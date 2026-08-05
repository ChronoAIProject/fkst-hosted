#!/usr/bin/env node
// FKST Evolution CLI — fingerprints, manifest assembly, and the convergence decision.
//
//   evolution fingerprint [--rev <rev>]
//   evolution build-manifest [--rev <rev>] [--write]
//   evolution verify [--rev <rev>] [--github]
//
// All diagnostics go to stderr; stdout carries only the machine-readable result,
// so `evolution verify --rev HEAD | jq .verdict` is a supported usage.

import { execFile } from 'node:child_process';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { promisify } from 'node:util';
import { buildManifest, presentManagedFiles } from './build.ts';
import { parseConfig } from './config.ts';
import { evaluateConvergence } from './converge.ts';
import {
  inputFingerprint, generatorPinnedFingerprint, generatorEnvFingerprint,
  outputFingerprint, readManagedOutputs, sourceFingerprints, directoryTreeFingerprint,
} from './fingerprints.ts';
import { GhCliGitHubPort } from './github.ts';
import { readTree } from './gittree.ts';
import { log, setLevel, type Level } from './log.ts';
import { manifestProjection, parseManifest } from './manifest.ts';
import { parsePlan } from './plan.ts';
import { CONFIG_PATH } from './selector.ts';
import { detectToolchain, SCHEMA_VERSIONS } from './build.ts';

const execFileAsync = promisify(execFile);

interface Args {
  command: string;
  rev: string;
  write: boolean;
  github: boolean;
}

function parseArgs(argv: string[]): Args {
  const args: Args = { command: argv[0] ?? 'help', rev: 'HEAD', write: false, github: false };
  for (let i = 1; i < argv.length; i += 1) {
    const flag = argv[i];
    if (flag === '--rev') args.rev = argv[++i];
    else if (flag === '--write') args.write = true;
    else if (flag === '--github') args.github = true;
    else if (flag === '--log-level') setLevel(argv[++i] as Level);
    else throw new Error(`unknown argument: ${flag}`);
  }
  return args;
}

async function repoRoot(): Promise<string> {
  const { stdout } = await execFileAsync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' });
  return stdout.trim();
}

/** Read a single file out of the Git tree at a revision. */
async function readFromTree(root: string, rev: string, path: string): Promise<string> {
  const entries = await readTree(root, rev, (p) => p === path);
  if (entries.length === 0) throw new Error(`${path} not found at ${rev}`);
  return entries[0].content.toString('utf8');
}

async function loadContext(root: string, rev: string) {
  const config = parseConfig(await readFromTree(root, rev, CONFIG_PATH));
  if (!config.enabled) log.warn('config has enabled: false — Evolution is disabled for this repository');
  const planText = await readFromTree(root, rev, 'tools/evolution/artifact-plan.yaml');
  const plan = parsePlan(planText);
  const { stdout } = await execFileAsync(
    'git', ['remote', 'get-url', 'origin'], { cwd: root, encoding: 'utf8' }
  );
  const match = /github\.com[:/]([^/]+\/[^/.]+)(\.git)?$/.exec(stdout.trim());
  if (!match) throw new Error(`cannot derive owner/repo from origin: ${stdout.trim()}`);
  return { config, plan, repository: match[1] };
}

async function cmdFingerprint(root: string, rev: string): Promise<void> {
  const { config, plan } = await loadContext(root, rev);
  const source = await sourceFingerprints(root, config, rev);
  const packages = await Promise.all(
    plan.generator.packages.map(async (pkg) => ({
      ref: pkg.ref.replace('{commit}', source.observedHead),
      treeFingerprint: await directoryTreeFingerprint(root, source.observedHead, pkg.directory),
    }))
  );
  const pinned = generatorPinnedFingerprint({
    manifestRef: plan.generator.manifestRef.replace('{commit}', source.observedHead),
    packages,
    schemaVersions: SCHEMA_VERSIONS,
    generatorEpoch: config.generatorEpoch,
  });
  const toolchain = await detectToolchain(root);
  const managed = await readManagedOutputs(root, source.observedHead);

  process.stdout.write(
    `${JSON.stringify(
      {
        observedHead: source.observedHead,
        productRelevantFingerprint: source.productRelevant,
        coverageFingerprint: source.coverage,
        generatorPinnedFingerprint: pinned,
        generatorEnvFingerprint: generatorEnvFingerprint({
          engineVersion: plan.generator.engineVersion,
          model: plan.generator.model,
          toolchain,
        }),
        inputFingerprint: inputFingerprint(source.productRelevant, pinned),
        productRelevantFileCount: source.productRelevantPaths.length,
        coverageFileCount: source.coveragePaths.length,
        managedOutputFileCount: managed.length,
      },
      null,
      2
    )}\n`
  );
}

async function cmdBuildManifest(root: string, rev: string, write: boolean): Promise<void> {
  const { config, plan, repository } = await loadContext(root, rev);
  const manifest = await buildManifest({
    repoRoot: root,
    config,
    plan,
    revision: rev,
    repository,
    // Fixed rather than "now" when supplied, so a rebuild of the same input is
    // byte-identical. Without it, `updatedAt` alone changes the manifest
    // projection and therefore the output fingerprint on every run.
    timestamp: process.env.FKST_EVOLUTION_TIMESTAMP ?? new Date().toISOString(),
  });
  const text = `${JSON.stringify(manifest, null, 2)}\n`;
  if (write) {
    const target = join(root, '.fkst/evolution/manifest.json');
    await writeFile(target, text, 'utf8');
    log.info('manifest written', { path: target });
  } else {
    process.stdout.write(text);
  }
}

async function cmdVerify(root: string, rev: string, useGitHub: boolean): Promise<void> {
  // The artifact repository comes from the manifest, not from the local remote:
  // condition 6 must ask about the repository the manifest claims, so a wrong
  // local remote cannot make a divergent sync PR invisible.
  const { config, plan } = await loadContext(root, rev);
  const manifest = parseManifest(await readFromTree(root, rev, '.fkst/evolution/manifest.json'));

  const source = await sourceFingerprints(root, config, rev);
  const packages = await Promise.all(
    plan.generator.packages.map(async (pkg) => ({
      ref: pkg.ref.replace('{commit}', manifest.source.observedHead),
      treeFingerprint: await directoryTreeFingerprint(root, manifest.source.observedHead, pkg.directory),
    }))
  );
  const pinned = generatorPinnedFingerprint({
    manifestRef: plan.generator.manifestRef.replace('{commit}', manifest.source.observedHead),
    packages,
    schemaVersions: SCHEMA_VERSIONS,
    generatorEpoch: config.generatorEpoch,
  });
  const currentInput = inputFingerprint(source.productRelevant, pinned);

  // Re-derive the output fingerprint from bytes, never from the manifest field.
  const managed = await readManagedOutputs(root, rev);
  const assets = manifest.artifacts
    .filter((a) => a.release)
    .map((a) => ({ tag: a.release!.tag, asset: a.release!.asset, contentHash: a.contentHash }));
  const currentOutput = outputFingerprint(managed, assets, manifestProjection(manifest));

  const report = await evaluateConvergence({
    repoRoot: root,
    config,
    manifest,
    currentInputFingerprint: currentInput,
    currentOutputFingerprint: currentOutput,
    observedHead: source.observedHead,
    presentFiles: await presentManagedFiles(root, rev),
    github: useGitHub ? new GhCliGitHubPort() : undefined,
  });

  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  // A non-zero exit for anything short of full convergence: this command is
  // meant to be usable as a gate, and "pending" is not "passed".
  if (report.verdict !== 'CONVERGED') process.exitCode = report.verdict === 'NOT_CONVERGED' ? 1 : 2;
}

async function main(): Promise<void> {
  const args = parseArgs(process.argv.slice(2));
  const root = await repoRoot();
  switch (args.command) {
    case 'fingerprint':
      return cmdFingerprint(root, args.rev);
    case 'build-manifest':
      return cmdBuildManifest(root, args.rev, args.write);
    case 'verify':
      return cmdVerify(root, args.rev, args.github);
    default:
      process.stdout.write(
        'usage: evolution <fingerprint|build-manifest|verify> [--rev <rev>] [--write] [--github]\n'
      );
      process.exitCode = args.command === 'help' ? 0 : 1;
  }
}

main().catch((error) => {
  log.error('command failed', { error: error instanceof Error ? error.message : String(error) });
  process.exitCode = 1;
});
