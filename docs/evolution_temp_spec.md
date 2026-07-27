# FKST Evolution: GitHub-Native Continuous Product Evolution

Status: Draft for discussion — revision 2

Date: 2026-07-24, revised 2026-07-27

Intended audience: FKST maintainers, package authors, hosted-control-plane
maintainers, repository owners, product owners, and security reviewers

This document is a temporary design specification. It defines a proposed
system and does not describe functionality that is already implemented unless
an existing FKST behavior is explicitly identified as such.

### Changes in revision 2

Revision 2 responds to a design review. The substantive changes:

| Change | Sections |
| ------ | -------- |
| **Single root.** All Evolution output moved under `.fkst/evolution/`. The write boundary is a control-plane prefix comparison that configuration cannot widen, replacing an owner-supplied managed-path set. | 12.1, 12.1.1, 12.1.2, 12.3, 13.2, 13.3.1, 25.8 |
| **Split fingerprints.** Six fingerprints replace four. Cycle admission and merge staleness follow a *product-relevant* fingerprint; a *coverage* fingerprint records provenance without launching cycles. Generator inputs split into repo-pinnable and deployment-environment halves. | 17.1, 17.3, 17.4, 17.5, 21.4, 32.3 |
| **Convergence is re-derived, not read.** A canonical manifest projection now MUST enter the output fingerprint, and convergence conditions 3-4 are re-derived from repository state and a controller-published check run rather than from status fields the generator wrote. | 16.2, 17.6, 17.7 |
| **The merge gate is a check run.** No native GitHub merge mechanism can satisfy a pre-merge freshness test, so the gate is an Evolution-owned required check plus a `sha`-pinned fallback. Evolution PRs are explicitly excluded from the generic FKST auto-merge hook. | 21.5.1, 21.5.2 |
| **Token containment restated to what tokens can do.** Installation tokens have no ref scope. The sandbox holds `contents: write` and never `pull_requests: write`; a required branch ruleset bounds the residual reach. | 25.1, 25.2.1, 25.5, 25.5.1 |
| **Capability identity, owner brake, companion homes.** Opaque IDs with explicit rename/merge/split relations; a suppression latch so closing a sync PR actually stops the loop; an explicit table of which repository hosts which resource. | 12.4.1, 14.2.1, 14.2.2, 26.2.1, 27.3 |
| **Rollout inverted.** The convergence oracle ships first, alone, writing nothing. | 37 |

Open questions 6 and 7 are answered in the body; 16 through 19 are new.

## Document map

- Sections 1-9 define the problem, purpose, constraints, principles, and terms.
- Sections 10-14 define component boundaries, placement, configuration, and the
  product model.
- Sections 15-17 define change records, the manifest, fingerprints, and
  convergence.
- Sections 18-22 define events, PR preview, singleton reconciliation, branches,
  merging, and loop prevention.
- Sections 23-24 define artifacts and GitHub Release storage.
- Sections 25-27 define security, recovery, and autonomy policy.
- Sections 28-34 define component work, UI projection, operations,
  compatibility, and adoption.
- Sections 35-42 define tests, acceptance criteria, rollout, examples,
  alternatives, open questions, and final invariants.
- Appendices provide draft machine markers, a freshness example, a minimal
  proof, and current implementation touchpoints.

## 1. Executive summary

FKST Evolution is a proposed repository-level capability that continuously
keeps a project's product-facing materials aligned with the latest trusted
state of the project. It observes pull requests and the repository's current
default branch, maintains a compact machine-readable product model, and
produces or updates:

- user-facing documentation;
- product-operation agent skills;
- executable product journeys;
- demo screenshots;
- demo videos;
- release notes and product change records; and
- editable and rendered slide decks.

Evolution is not a database-backed service. GitHub is the only durable system
of record. The source repository, or an explicitly configured companion GitHub
repository, stores all durable configuration, model data, generated source,
coordination state, history, and artifacts. The control plane may keep
short-lived queues, caches, leases, and running sandboxes, but a complete loss
of that process-local state MUST NOT lose work or change the final outcome.

The system is level-triggered rather than event-sourced. A GitHub webhook is a
best-effort wake-up hint. On every reconciliation, Evolution reads current
GitHub state and determines whether the repository is converged. Startup and
periodic full resynchronization repair missed, duplicated, delayed, or
out-of-order webhook deliveries.

Evolution MUST serialize canonical artifact production per repository. A burst
of commits may be analyzed in one run, but every reachable change since the
last converged source revision remains covered. At most one canonical Evolution
work issue and one canonical Evolution pull request may be open for a source
repository at a time.

Pull request heads are untrusted. Pre-merge processing is read-only and holds
no GitHub, demo, or publication credential (section 19.2.1). Canonical documentation, skills, demos, media, and decks are
generated from a commit that has reached the trusted default branch. This
separation permits useful PR impact feedback without executing contributor code
under a repository write token or demo credentials.

The central implementation concept is a repository-native materialized view:

```text
authoritative product source + owner intent + generator version
                              |
                              v
                 structured product observation
                              |
          +-------------------+-------------------+
          |         |         |         |         |
        docs      skills   screenshots  video   slides
                              |
                              v
         versioned Git files + GitHub Release assets
```

The system checks the complete managed product surface after each relevant
change, but only rewrites affected artifacts. A release event or explicit full
rebuild request may force regeneration of every configured artifact.

## 2. Normative language

The key words MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT, SHOULD, SHOULD NOT,
RECOMMENDED, MAY, and OPTIONAL in this document are to be interpreted as
normative requirements when written in uppercase.

Examples and proposed schemas are normative only where the surrounding text
says they are required. Field names marked as provisional may change before
implementation.

## 3. Background

### 3.1 Current FKST operating model

FKST runs package-driven coding sessions against GitHub repositories. A trigger
issue declares the session's package set and routing configuration. Open issues
with matching work labels form the session's work queue. A session performs
work in a sandbox, reports through GitHub, and normally produces a pull request.

The hosted control plane is intentionally durable-datastore-free. GitHub issues,
comments, labels, branches, pull requests, and repository files express durable
desired state and durable acknowledgements. Running Kubernetes pods or
OpenSandbox sandboxes express live runtime state. In-memory queues accelerate
reconciliation but do not establish correctness.

Current sessions are optimized for independent work items. Multiple open work
issues can be handled in parallel, with each issue producing an independent
pull request. That behavior is valuable for unrelated development tasks but is
not safe for Evolution artifacts, which share product models, documentation
indexes, screenshots, and release narratives. Evolution therefore requires a
single coalescing lane per repository.

FKST already reserves `.fkst/packages/` in a target repository as a repo-local
workflow catalog. Evolution extends the `.fkst/` namespace with
`.fkst/evolution/`; it does not replace or reinterpret `.fkst/packages/`.

### 3.2 Product continuity problem

Product-facing artifacts usually evolve at different speeds:

- application behavior changes in code;
- tests describe only selected behavior;
- documentation becomes stale;
- agent instructions retain obsolete paths or API fields;
- screenshots show previous layouts;
- demo videos stop matching current flows;
- release notes omit product implications; and
- decks reuse old claims and imagery.

The problem is not simply that artifact generation is manual. The deeper
problem is that each artifact often reconstructs its own understanding of the
product. Five independently generated outputs can disagree even when generated
on the same day.

Evolution addresses this by maintaining one structured, evidence-backed product
observation and deriving multiple artifact types from it. Owner-authored intent
is kept separate from agent-observed behavior so that the system cannot silently
turn an inference into product strategy.

### 3.3 Why a conventional documentation bot is insufficient

A documentation bot normally reacts to a diff and edits prose. Evolution has a
broader responsibility:

- determine whether a change is product-facing;
- identify affected capabilities and user journeys;
- preserve a semantic product-change history;
- verify current behavior against code, tests, schemas, and a running product;
- update several mutually dependent artifact formats;
- determine which artifacts are stale even when no source file names match;
- execute safe demo journeys against trusted code;
- retain provenance tying every output to an exact source state; and
- recover after missed events without a private event database.

These requirements make Evolution a reconciliation workflow rather than a text
generation task.

## 4. Purpose

Evolution exists to give repository and product owners an automatically
maintained, inspectable answer to four questions:

1. What can users do with the product now?
2. What product-visible behavior changed, when, and why?
3. Which public artifacts represent the current product accurately?
4. What evidence supports each claim, instruction, screenshot, video, or slide?

The system SHOULD reduce ongoing artifact maintenance to exception handling and
editorial review. After bootstrap, an enabled repository SHOULD converge
automatically after default-branch changes without requiring a person to open a
new Evolution issue for each commit.

## 5. Goals

Evolution has the following goals:

1. Detect every relevant pull request update and every change that reaches the
   repository's current default branch.
2. Remain correct when webhook events are lost, duplicated, reordered, or
   delivered after a restart.
3. Store all durable Evolution data in a GitHub repository.
4. Maintain a structured product model with stable capability and journey
   identifiers.
5. Produce mutually consistent documentation, skills, screenshots, videos,
   release narratives, and slide decks.
6. Track artifact freshness and provenance without a central registry.
7. Serialize canonical updates so competing runs cannot produce conflicting
   product projections.
8. Support autonomous operation with branch-protection-respecting merge policy.
9. Keep untrusted pull request execution separated from privileged post-merge
   production.
10. Permit a complete Evolution deployment restart without data migration or
    replay from a private event log.
11. Let owners retain product intent, terminology, publication policy, and
    artifact ownership boundaries under version control.
12. Keep artifact source editable and portable outside FKST.
13. Work with an application repository directly or with an owner-selected
    companion GitHub repository.
14. Make uncertainty and failed verification explicit rather than converting
    them into confident product claims.

## 6. Non-goals

Evolution is not intended to:

- replace Git, GitHub issues, pull requests, branch protection, or release
  management;
- create a private durable queue, project database, vector database, or hosted
  artifact registry;
- preserve every webhook payload;
- guarantee exactly one agent invocation per commit;
- publish unreviewed marketing claims inferred only from code;
- execute untrusted pull request code with repository write credentials,
  production data, or demo secrets;
- continuously rewrite arbitrary product source code as part of artifact
  synchronization;
- replace ordinary unit, integration, accessibility, or end-to-end tests;
- guarantee byte-identical LLM output for the same input;
- store credentials, tokens, raw prompts, or private runtime logs in the source
  repository;
- use GitHub Actions artifacts as durable storage; or
- require repository owners to use one documentation framework, skill format,
  presentation tool, or frontend test framework.

Evolution MAY discover source defects or missing automation. Those findings
SHOULD become separate FKST development work items and MUST NOT silently expand
the Evolution pull request beyond its configured ownership boundary.

## 7. Hard constraints

### 7.1 Durable-store constraint

GitHub repository resources are the only permitted durable store for Evolution.
For this specification, repository resources include:

- Git commits, trees, blobs, branches, and tags;
- repository files on ordinary branches;
- issues, issue comments, assignees, and labels;
- pull requests, pull request comments, and reviews;
- commit statuses or check results when permission is granted;
- GitHub Releases and Release assets; and
- resources in a configured companion GitHub repository.

No SQL, document, key-value, graph, vector, or event database is permitted.
Evolution artifacts MUST NOT depend on external object storage or Git LFS.

Short-lived runtime state is permitted when correctness does not depend on it.
Examples include:

- bounded in-memory reconcile queues;
- token and contents caches;
- debounce timers;
- a leader-election lease;
- a running sandbox filesystem;
- temporary media-rendering files; and
- retry counters held by a current process.

After all such state is destroyed, the next full reconciliation MUST be able to
derive the required action from GitHub and live sandbox discovery alone.

### 7.2 Default branch as canonical trust boundary

The current default branch is the canonical product source unless configuration
explicitly chooses another protected source branch. Pull request heads are
untrusted until merged. Canonical executable verification and publication MUST
use an exact commit reachable from the trusted source branch.

### 7.3 GitHub-native auditability

Every durable decision MUST be inspectable in normal GitHub resources. A
repository owner MUST be able to determine:

- which source commit was observed;
- which generator packages and tool versions were resolved;
- which capabilities and journeys were affected;
- which artifacts were updated;
- which verification steps passed or failed;
- why a run did not merge; and
- whether a later run superseded it.

### 7.4 Idempotent convergence

The same repository state may be reconciled any number of times. Reconciliation
MUST become a no-op once authoritative input, configured generator versions,
and managed output match the committed Evolution manifest.

### 7.5 Owner-controlled boundaries

The owner MUST declare which paths Evolution owns. Evolution MUST NOT modify a
path outside those boundaries. Human-authored product intent MUST have a
separate ownership class and MUST NOT be rewritten automatically.

## 8. Design principles

### 8.1 Desired state, observed state, and action

Evolution follows a controller model:

```text
desired state = current GitHub source + owner policy + generator revision
observed state = committed manifest + managed files + GitHub work resources
action         = the smallest safe operation that makes observed match desired
```

### 8.2 Events are hints, state is truth

Webhook delivery reduces latency. It is never the sole record that a change
occurred. Reconciliation always queries current branch or pull request state and
never trusts an event's stale head SHA without confirmation.

### 8.3 Detect each change, batch execution

Evolution distinguishes detection from execution. Each reachable change since
the last converged source revision MUST be considered. Several changes MAY be
processed in one session and one pull request.

### 8.4 Inspect all, rewrite affected outputs

After a relevant source change, Evolution SHOULD check the entire managed
product surface for consistency. It SHOULD only rewrite artifacts whose inputs,
dependencies, verification evidence, or generated bytes changed. A configured
full rebuild may deliberately regenerate every artifact.

### 8.5 One product model, multiple renderers

Artifact producers MUST consume a shared capability, journey, change, and
evidence model. A documentation renderer and a slide renderer MUST NOT derive
independent product facts from the repository without recording their findings
in the shared model.

### 8.6 Evidence before assertion

A factual product claim SHOULD reference one or more evidence sources, such as:

- a passing unit, integration, contract, or end-to-end test;
- an API or command schema;
- an application route or user-visible control;
- a successful executable journey;
- a migration or compatibility declaration;
- a merged pull request; or
- an owner-authored statement.

Unsupported inferences MUST be marked unverified or raised for owner review.

### 8.7 Safe autonomy

Autonomy means that the system detects drift, produces updates, verifies them,
and advances them according to repository policy without routine human
coordination. It does not mean direct pushes, permission bypass, or execution of
untrusted code with privileged credentials.

## 9. Terminology

| Term                  | Meaning                                                                                            |
| --------------------- | -------------------------------------------------------------------------------------------------- |
| Source repository     | The GitHub repository whose product is being observed                                              |
| Artifact repository   | The GitHub repository receiving Evolution state and artifacts; normally the source repository      |
| Companion repository  | An optional separate artifact repository selected by the owner                                     |
| Trusted source branch | Normally the source repository's current default branch                                            |
| Authoritative input   | Owner-controlled or product-source content from which artifacts are derived                        |
| Managed output        | A file or artifact that Evolution is authorized to rewrite                                         |
| Product intent        | Human-owned audience, terminology, positioning, promises, and constraints                          |
| Product observation   | Agent-maintained structured description of current capabilities and journeys                       |
| Capability            | A stable, user-relevant product function with evidence and lifecycle state                         |
| Journey               | A user goal expressed as ordered, verifiable product interactions                                  |
| Change record         | A semantic description of product-visible changes associated with a source commit                  |
| Artifact              | A generated document, skill, image, video, presentation, or release narrative                      |
| Artifact source       | Editable, diffable files from which an artifact may be rendered                                    |
| Rendered artifact     | A generated binary such as MP4, PDF, or PPTX                                                       |
| Input fingerprint     | Hash over product-relevant source paths plus repo-pinnable generator inputs (section 17.5)         |
| Coverage state        | Observed commit range recorded for provenance; does not admit a cycle (section 17.5)               |
| Output fingerprint    | Hash over everything Evolution wrote, plus a canonical manifest projection (section 17.6)          |
| Converged             | Input and output fingerprints match the committed manifest and no required verification is pending |
| Evolution cycle       | One serialized attempt to move a repository from observed state to desired state                   |
| Sync issue            | The single open GitHub work issue representing a canonical Evolution cycle                         |
| Sync PR               | The single open pull request carrying canonical Evolution updates                                  |
| PR preview            | Read-only impact analysis for an unmerged pull request                                             |

## 10. System context and responsibility boundaries

### 10.1 Hosted control plane

The hosted control plane SHOULD own only generic orchestration concerns:

- signature verification and classification of GitHub webhook events;
- repository reconciliation wake-up;
- startup and periodic full resynchronization;
- discovery of Evolution enrollment and current GitHub state;
- enforcement of one canonical Evolution lane per repository;
- creation or reopening of a coalesced sync issue;
- dynamic resolution of the current default branch;
- least-privilege credential minting;
- safe merge-policy enforcement;
- projection of GitHub-native Evolution state to a user-facing dashboard; and
- cleanup of runtime resources.

The hosted control plane MUST NOT understand how to write a user guide, operate
Playwright, compose a product skill, edit video, or render slides.

### 10.2 FKST packages

Evolution packages SHOULD own domain behavior:

- repository change analysis;
- capability and journey extraction;
- evidence collection;
- documentation maintenance;
- skill generation and conformance testing;
- deterministic demo environment preparation;
- screenshot and video capture;
- slide source generation and rendering;
- cross-artifact consistency checks; and
- Evolution manifest construction.

Packages SHOULD be composable through an FKST manifest so owners can enable
only the artifact types they need.

### 10.3 Source repository

The source repository owns:

- product source and tests;
- Evolution enrollment and configuration;
- human-authored product intent;
- current machine-readable product observation when no companion is used;
- generated artifact source when no companion is used;
- durable change and provenance records;
- the sync issue and sync PR; and
- repository policy governing required checks and merge behavior.

### 10.4 Companion repository

A companion repository MAY own product observation, artifact source, and
rendered artifacts for owners that do not want generated content in the source
repository. The source repository MUST retain enough configuration to identify
the companion repository, and every companion artifact MUST record the exact
source repository and source commit it represents.

The GitHub App MUST have authority on both repositories. Cross-repository
updates are not atomic; the manifest in the artifact repository is canonical
for artifact convergence, while source-repository configuration is canonical
for enrollment and destination selection.

### 10.5 GitHub

GitHub provides the durable substrate:

- Git history stores source, structured state, and editable outputs;
- issues and labels store work coordination and durable status;
- pull requests store proposed canonical transitions and reviews;
- comments store bounded human-readable diagnostics and machine markers;
- branch protection and merge queues enforce repository policy; and
- Releases store large rendered artifacts.

## 11. High-level architecture

```mermaid
flowchart TD
    A[GitHub push or pull_request webhook] -->|best-effort hint| B[Reconcile dispatcher]
    C[Startup and periodic full resync] --> B
    B --> D[Read current GitHub state]
    D --> E{PR head or trusted branch?}
    E -->|Unmerged PR| F[Read-only impact preview]
    E -->|Trusted branch| G[Compute input and output fingerprints]
    G --> H{Converged?}
    H -->|Yes| I[No-op and repair stale status]
    H -->|No| J[Ensure one sync issue]
    J --> K[Run Evolution package manifest]
    K --> L[Open or update one sync PR]
    L --> M[Verify and re-read trusted head]
    M -->|Head advanced| K
    M -->|Current and green| N[Protected merge]
    N --> O[Manifest and outputs become canonical]
    O --> G
```

The control plane can lose `B` entirely. `C` will reconstruct pending work from
GitHub and live runtime state.

## 12. Durable data placement

### 12.1 Default placement

Evolution writes to exactly one root: `.fkst/evolution/` in the artifact
repository. Structured state, human-owned intent, and every generated artifact
source live under that root. Large rendered binaries are stored as Release
assets in the same repository and referenced by `manifest.json`.

```text
.fkst/
  packages/                         existing FKST workflow catalog (NOT Evolution)
  evolution/
    config.yaml                     owner-controlled policy            [human]
    intent/
      product.md                    owner-controlled narrative intent  [human]
      overrides.yaml                owner-controlled protected facts   [human]
    observed/
      capabilities.yaml             agent-maintained product observation
      journeys.yaml                 journey metadata and evidence references
    changes/
      <yyyy>/<sha[0:2]>/<sha>.yaml  semantic product change record
    manifest.json                   convergence and artifact provenance
    docs/                           generated user-facing documentation
    skills/                         generated product-operation skills
    journeys/                       executable demo specifications
    screenshots/                    small current screenshots
    slides/                         editable presentation source
```

Rendered MP4, PDF, PPTX, and other large binaries SHOULD be stored in GitHub
Releases and referenced by `manifest.json`.

Change records are sharded by year and commit-SHA prefix. A flat directory
accumulates one entry per covered commit for the life of the repository and is
re-enumerated on every reconcile by section 17.6.

#### 12.1.1 Single-root confinement

The single root is a structural safety boundary, not a filing convention. The
control plane MUST enforce the following independently of any
repository-supplied configuration:

1. Evolution MUST NOT create, modify, or delete any path outside
   `.fkst/evolution/`.
2. Evolution MUST NOT modify `.fkst/evolution/config.yaml` or any path under
   `.fkst/evolution/intent/` **in a sync PR**. Sections 12.3 and 14.4 permit
   Evolution to *propose* intent changes; such a proposal MUST be a separate
   pull request on its own branch, never merged by autonomous policy, and it is
   never part of the managed-output cycle. The section 25.8 confinement check
   applies to the sync PR and rejects these paths there unconditionally.
3. Evolution MUST NOT modify any path under `.fkst/packages/`.

Rule 1 is a fixed prefix comparison. It is not derived from `managedOutputs`,
and configuration cannot widen it. `config.yaml` selects which subtrees beneath
the root are produced; it can never extend the root itself. This is what makes
security objective 25.1(3) true rather than aspirational: repository content
selects among subtrees inside a boundary it cannot move.

#### 12.1.2 Consumption

Generated artifacts live under a dot-directory, which conventional tooling does
not discover by name. Consumers SHOULD be pointed at the root through their own
configuration — a test runner's `testDir`, a documentation site's source path,
an agent-skill loader root.

Copying generated files out to conventional locations creates a second
maintained copy and MUST NOT be performed by Evolution. A repository owner MAY
maintain such a publication step as ordinary human-owned automation outside the
Evolution lane. Symlinks from conventional paths into the root are permitted
and are fingerprint-safe under section 17.2, but do not survive Windows
checkouts.

### 12.2 What MUST NOT be stored under `.fkst/evolution/`

The directory MUST NOT contain:

- repository, GitHub App, LLM, or demo-environment credentials;
- raw private user or production data;
- raw LLM prompts or transcripts;
- unredacted runtime logs;
- browser profiles, cookies, or authenticated storage state;
- package-manager, compiler, or browser caches;
- temporary media frames;
- large videos, PDFs, or presentation binaries; or
- a private copy of webhook payloads.

### 12.3 Data ownership classes

Ownership is derived from the path prefix, not from configuration:

| Class                | Path                                        | Owner              | Evolution behavior                    |
| -------------------- | ------------------------------------------- | ------------------ | ------------------------------------- |
| Authoritative source | everything outside `.fkst/evolution/`       | Product developers | Read, never write                     |
| Human intent         | `.fkst/evolution/config.yaml`, `intent/**`  | Repository owner   | Read, never write; may propose in a separate reviewed PR |
| Observed model       | `.fkst/evolution/{observed,changes}/`, `manifest.json` | Evolution | Update through sync PRs |
| Managed output       | all other paths under `.fkst/evolution/`    | Evolution          | Regenerate or repair through sync PRs |

There is no `shared/manual` class and no mixed-ownership file. Because the
boundary is a prefix comparison (section 12.1.1), the previous draft's rule
that "a path MUST NOT be both human-owned and Evolution-managed" is now
structurally guaranteed rather than a validation obligation, and the machine-
marker mechanism for generated blocks inside human-owned files is no longer
required for Evolution's own outputs.

An owner who wants generated content to appear at a conventional path uses
section 12.1.2 consumption, not mixed ownership.

### 12.4 Companion repository placement

When `artifactRepository` names a companion repository, the source repository
retains only enrollment and destination configuration. The companion repository
uses the same `.fkst/evolution/` schema and stores all generated source and
Release assets.

The companion manifest MUST include:

- source repository full name;
- source default branch resolved for the run;
- exact source commit SHA;
- source product-relevant, coverage, and input fingerprints;
- companion output fingerprint; and
- generator pinned and environment fingerprints.

The Git commit containing the manifest is derived from GitHub when the manifest
is read. It is deliberately not embedded in the manifest itself, which would
create a self-referential commit identity.

#### 12.4.1 Which repository hosts which resource

When source and artifact repositories differ, each coordination resource has
exactly one home. Implementations MUST use this table; leaving it implicit
produces a split-brain lane.

| Resource                       | Repository | Rationale                                              |
| ------------------------------ | ---------- | ------------------------------------------------------ |
| `config.yaml`, enrollment      | Source     | Enrollment is a property of the observed product       |
| Trigger issue                  | Source     | Follows existing FKST session registration             |
| Sync issue                     | Source     | Owners watch the product repository                    |
| Sync PR, sync branch           | Artifact   | It carries the artifact commits                        |
| `intent/**`, `observed/**`, `changes/**`, `manifest.json` | Artifact | Convergence is decided where the outputs live |
| Release assets                 | Artifact   | Co-located with the manifest that references them      |
| Merge gate check run           | Artifact   | Must gate the PR it protects                           |

Section 17.7 condition 6 is evaluated against the artifact repository. The
singleton tuple of section 20.1 spans the pair: one lane per
`(source repository, artifact repository, trusted source branch)`, with the
sync issue in the source repository holding the lane's identity and the sync PR
in the artifact repository carrying its content. Branch protection honored
under section 21.5 is the **artifact** repository's, because that is where the
merge occurs.

## 13. Enrollment and configuration

### 13.1 Enrollment

A repository is enrolled when both of the following exist:

1. an open FKST trigger issue whose packages or manifest include the Evolution
   workflow; and
2. `.fkst/evolution/config.yaml` on the trusted source branch.

An installation-time bootstrap MAY create a draft trigger issue and a baseline
configuration PR. It MUST NOT silently enable autonomous merging without an
owner-selected policy.

An App-authored bootstrap trigger is subject to existing FKST trigger
attribution: a bot-authored trigger's effective creator is its **sole
assignee**, who must be a deployment global admin or hold repository admin or
maintain permission. Zero or multiple assignees are not attributable and the
trigger is rejected with `fkst-trigger-unauthorized` before its body is parsed.
A bootstrap that does not assign exactly one qualifying creator will therefore
never enroll the repository.

### 13.2 Draft configuration schema

The following schema is illustrative and intentionally explicit:

```yaml
schemaVersion: 1
enabled: true

source:
  branch: "@default"

  # Paths that can plausibly change the PRODUCT SURFACE. This set drives cycle
  # admission and the meaning of publication.requireCurrentSource (section
  # 17.5). It is deliberately NOT "**": a comment typo must not launch a cycle.
  #
  # The list below is ILLUSTRATIVE, not a shipped default. No default set is
  # specified by this draft: a too-narrow guess misses real product changes
  # invisibly, and a too-broad one reproduces the per-commit cost the split
  # exists to remove. Enrollment REQUIRES an explicit declaration (section
  # 13.3), and rollout Phase 1 measures real repositories to determine whether
  # a defensible default exists at all (open question 16).
  productRelevant:
    include:
      - "src/**"
      - "app/**"
      - "frontend/**"
      - "backend/**"
      - "openapi.json"
      - "**/*.proto"
      - "migrations/**"
    exclude:
      - "**/*_test.*"
      - "**/*.test.*"
      - "**/testdata/**"

  # Everything else reachable on the trusted branch is still COVERED for
  # provenance (section 17.5) but does not by itself launch a cycle.
  # ".fkst/evolution/**" and ".fkst/packages/**" are removed unconditionally by
  # section 17.3 and must NOT be re-added here; "**" below is understood to be
  # taken after that removal, not before it.
  coverage:
    include:
      - "**"
    exclude:
      - ".git/**"

# Paths under .fkst/evolution/ are excluded from both fingerprints by section
# 17.3 unconditionally. Configuration cannot re-include them and cannot extend
# the write boundary of section 12.1.1.

artifactRepository: "."

intent:
  product: ".fkst/evolution/intent/product.md"
  overrides: ".fkst/evolution/intent/overrides.yaml"

# Each managed output selects a SUBTREE beneath .fkst/evolution/. There is no
# free-form path field: the subtree name is fixed by schema, so configuration
# can enable or disable a class but can never point it at another location.
managedOutputs:
  documentation: { enabled: true }        # -> .fkst/evolution/docs/
  skills:        { enabled: true }        # -> .fkst/evolution/skills/
  journeys:      { enabled: true }        # -> .fkst/evolution/journeys/
  screenshots:   { enabled: true }        # -> .fkst/evolution/screenshots/
  slides:        { enabled: true }        # -> .fkst/evolution/slides/
  video:         { enabled: true, storage: "github-release" }

locales:
  - "en"

triggers:
  pullRequestPreview: true
  defaultBranchPush: true
  releaseFullRebuild: true
  debounceSeconds: 60

publication:
  mode: "propose"                 # bootstrap default; see section 27.1
  requireCurrentSource: true      # evaluated against productRelevant only
  requireChecks: true
  allowDirectPush: false

  # Owner brake (section 26.2.1). Closing the sync PR without merging suppresses
  # regeneration for THAT input fingerprint until an input changes or the
  # suppression is cleared. "none" restores the previous always-retry behavior.
  onOwnerClose: "suppress-until-input-changes"   # none | suppress-until-input-changes
  suppressionLabel: "fkst-evolution-suppressed"

  # Bound on regeneration rounds within one cycle (section 20.2 step 17).
  # Exhausting either marks the PR BLOCKED rather than looping forever.
  maxRegenerationRounds: 5
  cycleDeadlineSeconds: 3600

# What to do when a managed output changed outside the Evolution lane
# (section 17.8). An integrity mismatch under section 17.7 condition 3 is
# always treated as "block" regardless of this setting.
drift:
  policy: "block"                 # block | repair | adopt

# Deliberate lever for regenerating when nothing in the repository changed but
# the generator environment did (section 17.4). Bumping it forces one cycle.
generatorEpoch: 1

retention:
  renderedSnapshots: 10           # THIS number is the deletion policy required
  preserveProductReleases: true   # by section 24.5; absent => report only

security:
  runPullRequestCode: false
  allowProductionData: false
  allowProductionCredentials: false
```

### 13.3 Configuration validation

Configuration validation MUST fail closed when:

- `schemaVersion` is unsupported;
- the configured source branch cannot be resolved;
- the artifact repository is malformed or inaccessible;
- `source.productRelevant` is absent, empty, or matches no path on the trusted
  branch — an empty product-relevant set silently disables all cycle admission;
- `source.productRelevant` or `source.coverage` names any path under
  `.fkst/evolution/` or `.fkst/packages/` in an `include` entry **explicitly**
  — that is, in a pattern that is not a general wildcard. A broad `"**"` is
  permitted and is simply narrowed by the unconditional removals of section
  17.3; an explicit `".fkst/evolution/docs/**"` is a request to re-include and
  fails closed;
- a `managedOutputs` entry carries a path, directory, or destination field —
  destinations are fixed by schema (section 13.2) and are not configurable;
- direct push is requested;
- a requested storage mode is not GitHub-native;
- a requested merge policy cannot honor required checks;
- `publication.mode` is `automerge-managed` while the merge gate of section
  21.5 is not installable on the artifact repository; or
- security policy requests privileged execution of an untrusted PR head.

Unknown fields SHOULD be rejected until the schema defines an extension
mechanism. Silent acceptance would let misspelled safety policy appear active.

#### 13.3.1 Configuration is not the safety boundary

Validation runs on owner-supplied data and is therefore an input check, not a
control. The write boundary of section 12.1.1 MUST be enforced in the control
plane at every commit, push, and merge, independently of whether configuration
validation ran, succeeded, or was bypassed. A configuration file that somehow
passes validation while requesting a write outside `.fkst/evolution/` MUST
still be refused at the point of write.

This is the distinction that makes security objective 25.1(3) — "repository
content cannot expand the agent's permissions or its write boundary" — true.
`config.yaml` is repository content; if it defined the boundary, that objective
would be self-contradictory.

### 13.4 Configuration changes

A configuration change is authoritative input and MUST trigger reconciliation.
Disabling a managed output class MUST NOT automatically delete its previously
generated subtree. Evolution SHOULD report it as released from management and
require an explicit cleanup policy or separate reviewed deletion.

Enabling a managed output class that was previously disabled is a widening
operation. The first cycle that writes into a newly enabled subtree MUST run
under `propose` regardless of the configured `publication.mode`, so the first
generation at that destination receives its own human merge. Subsequent cycles
use the configured mode.

Changing `artifactRepository` requires an explicit migration. The old manifest
remains historical evidence and MUST NOT be silently deleted.

## 14. Product model

### 14.1 Model purpose

The product model is a compact, structured observation that gives all artifact
renderers a common understanding. It is not intended to duplicate the complete
source tree or function as an application architecture database.

### 14.2 Capability schema

Each capability MUST have a stable identifier. Renaming a title MUST NOT create
a new identity. A draft entry is:

```yaml
schemaVersion: 1
capabilities:
  - id: "cap_7f3a"
    title: "CSV import"
    status: "available"
    summary: "Import structured records from a CSV file."
    audiences:
      - "workspace-admin"
    interfaces:
      - kind: "ui"
        entrypoint: "/imports/new"
      - kind: "api"
        entrypoint: "POST /api/v1/imports"
    limitations:
      - "Maximum upload size is 25 MiB."
    journeys:
      - "jny_4c81"
    evidence:
      - kind: "test"
        ref: "frontend/e2e/import.spec.ts"
      - kind: "schema"
        ref: "GET /openapi.json#/paths/~1api~1v1~1imports"
    introducedBy: "<commit-sha>"
    lastChangedBy: "<commit-sha>"
    verification: "passed"
```

Allowed lifecycle states SHOULD include:

- `planned`, only when explicitly owner-authored;
- `experimental`;
- `available`;
- `deprecated`;
- `removed`; and
- `unknown` for incomplete bootstrap analysis.

Evolution MUST NOT infer `planned` product commitments from branches, TODOs, or
unmerged pull requests.

This lifecycle vocabulary describes a **capability**. It is disjoint from the
artifact status vocabulary of section 16.3 (`current`, `stale`, `blocked`, …),
which describes a generated file. The two MUST NOT be mixed in one field, and
section 33.3's fail-on-unknown rule applies to each independently.

#### 14.2.1 Identity allocation

A capability identifier MUST be allocated once and never re-derived. Concretely:

1. Identifiers are opaque and MUST NOT be derived from the title, path, or
   summary, so that a rename cannot produce a different identifier by
   construction rather than by instruction.
2. New identifiers are allocated only when no existing entry in
   `observed/capabilities.yaml` describes the same product function. The prior
   model is a REQUIRED input to every generation, including a full rebuild
   (section 32.3).
3. When generation cannot match an observed capability to any current product
   function, it MUST mark that capability `unknown` and raise it for owner
   adjudication. It MUST NOT delete the entry or reallocate its identifier.

#### 14.2.2 Rename, merge, and split

`added`/`changed`/`deprecated`/`removed` (section 15.3) cannot express identity
transitions. Merging two capabilities into one can only be encoded as two
removals plus one addition, which generated release notes and the section 30.2
timeline then publish as "features removed" — a false public claim about a
product that lost nothing.

Change records MUST therefore support explicit relations:

```yaml
capabilities:
  renamed:
    - { id: "cap_7f3a", previousTitle: "CSV upload", title: "CSV import" }
  merged:
    - { into: "cap_7f3a", from: ["cap_2b91", "cap_5d04"] }
  split:
    - { from: "cap_7f3a", into: ["cap_9c17", "cap_1e60"] }
```

`removed` is reserved for a capability the product genuinely no longer offers.
Because a false removal is a user-visible product claim, a change record
containing `capabilities.removed` requires human review under section 27.3 and
MUST NOT auto-merge.

### 14.3 Journey schema

Journey metadata links product meaning to executable verification. Executable
code remains in the configured journey source directory.

```yaml
schemaVersion: 1
journeys:
  - id: "jny_4c81"
    title: "Import and validate customer records"
    audience: "workspace-admin"
    capabilities:
      - "cap_7f3a"
    executable:
      framework: "playwright"
      ref: ".fkst/evolution/journeys/csv-import.spec.ts"
    prerequisites:
      - "A synthetic workspace with import permission"
    captures:
      - id: "csv-mapping"
        kind: "screenshot"
      - id: "csv-import-demo"
        kind: "video"
    evidence:
      lastVerifiedSource: "<commit-sha>"
      result: "passed"
```

### 14.4 Product intent

Product intent is human-owned. It SHOULD cover:

- intended audiences;
- product purpose and value proposition;
- canonical terminology;
- prohibited or regulated claims;
- known non-goals;
- brand and accessibility requirements;
- expected documentation voice;
- demo-data constraints; and
- presentation audiences and confidentiality classes.

Evolution MAY propose changes to intent in a separately reviewable suggestion,
but MUST NOT merge those changes through autonomous managed-output policy.

### 14.5 Overrides

Overrides protect facts that cannot be reliably inferred. Example:

```yaml
schemaVersion: 1
protectedFacts:
  - id: "upload-limit"
    statement: "CSV uploads are limited to 25 MiB."
    evidence: "docs/architecture/import-limits.md"
    owner: "product"

terminology:
  forbidden:
    - "account owner"
  preferred:
    "account owner": "workspace administrator"

artifactRules:
  - match: ".fkst/evolution/slides/investor/**"
    requiresHumanReview: true
```

An observed contradiction with a protected fact MUST block publication of the
affected artifact and produce a clear review request.

## 15. Product change records

### 15.1 Purpose

Git history records implementation changes but does not reliably express their
product meaning. Evolution change records provide an append-only semantic layer
without creating a separate database.

### 15.2 Identity and file naming

The canonical path is:

```text
.fkst/evolution/changes/<yyyy>/<sha[0:2]>/<full-sha>.yaml
```

The path is sharded by year and commit-SHA prefix (section 12.1); the full SHA
remains the identity, and the shard is derived from it, so the location of a
record is always computable from its identity alone.

The source commit SHA is the identity. Reprocessing the same commit MUST update
the same proposed file rather than creating another record. Once merged, a
record SHOULD remain immutable except for schema migration or correction of a
demonstrable factual error.

For a squash-merged pull request, the default-branch squash commit is the source
identity and the record includes the pull request number and last observed PR
head. Individual commits that never become reachable from the trusted branch do
not receive canonical change records.

### 15.3 Draft schema

```yaml
schemaVersion: 1
sourceCommit: "<full-commit-sha>"
sourceParents:
  - "<parent-sha>"
pullRequest:
  number: 412
  url: "https://github.com/owner/project/pull/412"
  headCommit: "<last-pr-head-sha>"

classification: "product-change"
summary: "Workspace administrators can import and validate CSV records."

capabilities:
  added:
    - "cap_7f3a"      # opaque, allocated once (section 14.2.1)
  changed: []
  deprecated: []
  # `removed` means the product genuinely no longer offers it. It is NOT how a
  # rename, merge, or split is expressed — those use the relations below, and a
  # non-empty `removed` requires human review (sections 14.2.2, 27.3).
  removed: []
  renamed: []      # [{ id, previousTitle, title }]
  merged: []       # [{ into, from: [...] }]
  split: []        # [{ from, into: [...] }]

journeys:
  added:
    - "jny_4c81"
  changed: []
  removed: []
  renamed: []
  merged: []
  split: []

artifactImpact:
  documentation:
    - "docs.csv-import"
  skills:
    - "skill.csv-import"
  screenshots:
    - "screenshot.csv-mapping"
  videos:
    - "video.csv-import"
  slides:
    - "deck.release-update"

migrations: []
limitations:
  - "Maximum upload size is 25 MiB."

evidence:
  - kind: "pull-request"
    ref: "#412"
  - kind: "test"
    ref: "frontend/e2e/import.spec.ts"

verification: "passed"
```

### 15.4 Classification

Allowed classifications SHOULD include:

- `product-change`;
- `product-fix`;
- `deprecation`;
- `removal`;
- `migration`;
- `internal-only`;
- `documentation-only`;
- `generated-only`; and
- `uncertain`.

An `internal-only` or `generated-only` commit MAY have a small record when it is
needed to prove coverage, but implementations MAY instead track coverage only
through the manifest when no product semantics changed.

### 15.5 Coverage across batched commits

A single Evolution cycle may process a range of commits. It MUST walk every
reachable commit from the previous covered source revision, exclusive, through
the currently observed source revision, inclusive. Merge topology and pull
request metadata SHOULD be used to avoid presenting implementation merge commits
as independent user-facing releases.

If the previous revision is not an ancestor of the current revision, Evolution
MUST perform a full product comparison and record that history was rewritten.
It MUST NOT guess a linear range from timestamps.

## 16. Evolution manifest

### 16.1 Purpose

`.fkst/evolution/manifest.json` is the canonical compact convergence record. It
replaces the need for a database cursor or hosted artifact registry. It answers:

- which trusted source state was used;
- which generator revision produced the outputs;
- which product-relevant, coverage, generator-pinned, generator-environment,
  input, and output fingerprints were calculated;
- which commit range was covered;
- which artifacts exist and where;
- which capabilities and journeys each artifact represents; and
- whether verification passed.

### 16.2 Draft schema

```json
{
  "schemaVersion": 1,
  "source": {
    "repository": "owner/project",
    "branch": "main",
    "observedHead": "abc123fullsha",
    "previousCoveredHead": "def456fullsha",
    "historyRelation": "fast-forward",
    "productRelevantFingerprint": "sha256:...",
    "coverageFingerprint": "sha256:...",
    "inputFingerprint": "sha256:..."
  },
  "artifactRepository": {
    "repository": "owner/project",
    "branch": "main"
  },
  "generator": {
    "manifestRef": "owner/fkst-packages@resolved-sha:manifests/product-evolution.json",
    "packages": [
      "owner/fkst-packages@resolved-sha:packages/evolution-observer",
      "owner/fkst-packages@resolved-sha:packages/evolution-docs"
    ],
    "generatorEpoch": 1,
    "pinnedFingerprint": "sha256:...",

    "_comment": "everything below is provenance only and does NOT gate convergence (section 17.4)",
    "engineVersion": "<version>",
    "model": "<provider-and-model-id>",
    "toolchain": {
      "playwright": "<version>",
      "ffmpeg": "<version>",
      "slideRenderer": "<name-and-version>"
    },
    "envFingerprint": "sha256:..."
  },
  "outputFingerprint": "sha256:...",
  "verification": {
    "status": "passed",
    "verifiedAt": "2026-07-24T12:00:00Z",
    "checks": [
      {
        "id": "jny_4c81",
        "status": "passed",
        "evidence": ".fkst/evolution/journeys/csv-import.spec.ts"
      }
    ]
  },
  "artifacts": [
    {
      "id": "docs.csv-import",
      "kind": "documentation",
      "locale": "en",
      "audience": "workspace-admin",
      "capabilities": ["cap_7f3a"],
      "journeys": ["jny_4c81"],
      "sourceCommit": "abc123fullsha",
      "inputFingerprint": "sha256:...",
      "generatorPinnedFingerprint": "sha256:...",
      "repositoryPath": ".fkst/evolution/docs/csv-import.md",
      "contentHash": "sha256:...",
      "status": "current",
      "verification": "passed",
      "updatedAt": "2026-07-24T12:00:00Z"
    },
    {
      "id": "video.csv-import",
      "kind": "video",
      "locale": "en",
      "audience": "workspace-admin",
      "capabilities": ["cap_7f3a"],
      "journeys": ["jny_4c81"],
      "sourceCommit": "abc123fullsha",
      "inputFingerprint": "sha256:...",
      "generatorPinnedFingerprint": "sha256:...",
      "release": {
        "repository": "owner/project",
        "tag": "fkst-evolution/0123456789abcdef",
        "asset": "csv-import.sha256-abcd1234ef567890.mp4",
        "assetUrl": "https://github.com/owner/project/releases/download/..."
      },
      "contentHash": "sha256:...",
      "status": "current",
      "verification": "passed",
      "updatedAt": "2026-07-24T12:00:00Z"
    }
  ]
}
```

Every artifact entry carries the input and generator-pinned fingerprints
required by section 23.1; `sourceCommit` alone does not identify the generator
revision that produced the bytes. `assetUrl` is a convenience for readers and
is NOT authoritative: section 17.7 condition 3 re-derives the asset by
`repository` + `tag` + `asset` and re-hashes it, so a stale or rewritten URL
cannot make a missing artifact look present.

### 16.3 Artifact status

Artifact status values SHOULD include:

- `current`: generated from the manifest input and verified;
- `current-unverified`: generated from the manifest input but not fully tested;
- `stale`: known not to represent current input;
- `blocked`: generation cannot proceed until a declared condition is resolved;
- `failed`: an attempted generation or verification failed;
- `deprecated`: retained for compatibility but no longer preferred; and
- `removed`: represented as a tombstone when historical continuity is needed.

A manifest merged under autonomous policy MUST NOT claim `current` when a
required verification failed.

### 16.4 Manifest lifecycle

The manifest advances only when its corresponding sync PR merges. An issue
comment, open PR, successful local generation, uploaded draft Release asset, or
passing sandbox test is not canonical completion.

The manifest MUST NOT contain the SHA of the final Git commit that contains the
manifest. Consumers obtain that commit from the GitHub ref or contents response.
Embedding it would require changing the file, which would create a different
commit SHA indefinitely.

When the artifact repository differs from the source repository, the companion
manifest advances through a PR in the companion repository. The source commit
remains immutable provenance even if the source branch advances before that PR
merges.

### 16.5 Corruption handling

An absent manifest means the repository requires baseline generation. A
malformed or unsupported manifest MUST NOT be silently replaced under
autonomous merge policy. Evolution SHOULD open or update the sync issue with a
bounded diagnostic and propose a repair PR that preserves the invalid file for
review or history.

## 17. Fingerprints and convergence

### 17.1 Why commit SHA comparison is insufficient

Every merged Evolution PR creates a new trusted-branch commit. If convergence
were defined as `manifest.source.observedHead == currentBranchHead`, Evolution
would trigger itself indefinitely. A commit SHA also fails to detect a mutable
generator reference moving while source code remains unchanged.

Evolution therefore uses separate fingerprints. There are six, in two families:

| Fingerprint       | Covers                                        | Governs                          |
| ----------------- | --------------------------------------------- | -------------------------------- |
| `productRelevant` | `source.productRelevant` paths                | Cycle admission, merge staleness |
| `coverage`        | `source.coverage` paths                       | Provenance only                  |
| `generatorPinned` | Resolved package commits, prompts, schemas    | Cycle admission                  |
| `generatorEnv`    | Engine version, model id, tool versions       | Provenance only                  |
| `input`           | `productRelevant` + `generatorPinned` + config | Convergence condition 1         |
| `output`          | Everything Evolution wrote                    | Convergence condition 2          |

The split exists because the previous draft used a single input fingerprint
over `include: ["**"]`, which made "any byte of the repository changed" the
cycle-admission rule while section 21.4 simultaneously required "the source has
not moved since" as the merge rule. On an active repository those two never
both hold; on a quiet one they produce a merged Evolution PR per commit.

### 17.2 Canonical file hashing

For every fingerprinted repository file, implementations MUST:

1. use the Git tree at the exact revision rather than filesystem modification
   time;
2. normalize paths to slash-separated repository-relative UTF-8 names;
3. preserve Git file mode in the hash input;
4. hash raw blob bytes without line-ending conversion;
5. sort entries by path byte order; and
6. hash a length-delimited representation so path and content boundaries cannot
   collide.

A conceptual leaf is:

```text
leaf = SHA256(path_length || path || mode || content_length || content_bytes)
```

The tree fingerprint is SHA-256 over the ordered length-delimited leaves.

### 17.3 Source tree fingerprints

Two source fingerprints are computed over the trusted tree using the section
17.2 leaf construction.

`productRelevantFingerprint` covers files selected by
`source.productRelevant.include` minus `source.productRelevant.exclude`, plus
`config.yaml` and `intent/**`, which are always included.

In a companion setup this is a **two-repository computation**: product-relevant
paths and `config.yaml` are read from the source repository, and `intent/**`
from the artifact repository, per the section 12.4.1 homing table. Both
contribute to the one `productRelevantFingerprint`. Without this rule human
intent would stop being a convergence input whenever a companion is used —
an owner could rewrite product positioning and nothing would regenerate.

`coverageFingerprint` covers files selected by `source.coverage.include` minus
`source.coverage.exclude`.

Both are subject to these rules, which are unconditional and which
configuration MUST NOT override:

- every path under `.fkst/evolution/` is excluded from both fingerprints,
  **except** `config.yaml` and `intent/**`, which are included in
  `productRelevantFingerprint` only;
- every path under `.fkst/packages/` is excluded from both, because section
  33.4 classifies it as an independent workflow catalog;
- submodule identity is the recorded submodule commit, not an implicit clone of
  mutable remote content; and
- symlinks are hashed as Git symlink blobs and MUST NOT be followed outside the
  checkout.

The previous draft expressed exclusion twice — once as a prose rule here and
once as a `source.exclude` list in the configuration example — without saying
which prevailed. It is this section. The single-root layout of section 12.1
makes the rule a prefix comparison rather than a pattern set.

### 17.4 Generator fingerprints

The generator inputs are split by who controls them, because they converge
differently.

`generatorPinnedFingerprint` covers inputs a repository can pin, and
participates in convergence:

- every FKST manifest and package reference resolved to an immutable commit;
- package configuration and prompts;
- template and theme files not already covered by section 17.3;
- schema versions for capabilities, journeys, changes, and manifest; and
- `config.yaml`'s `generatorEpoch`.

`generatorEnvFingerprint` covers deployment facts the repository does not
control, and is recorded as provenance only:

- engine version;
- declared model provider and model identifier;
- renderer and media tool versions.

**Why the split.** In this deployment the model identifier is control-plane
configuration, not repository state. If it entered the convergence-bearing
fingerprint, a single operator model roll would change the input fingerprint of
every enrolled repository simultaneously, and section 18.4's startup full
resync would synchronize the resulting regeneration wave with the deploy. There
is no fleet-wide in-flight cap that would contain it — section 28.3's
`max_in_flight = 1` is per work-label lane, not per fleet.

An environment change therefore updates provenance without launching cycles.
When an operator judges that a generator change genuinely warrants
regeneration, the deliberate levers are the repository's `generatorEpoch` or an
explicit full rebuild (section 32.3), both of which are rate-limitable and
opt-in per repository. An implementation MUST NOT make `generatorEnv` a
convergence input without also specifying a fleet-wide rollout budget.

Secrets and secret values MUST NOT enter any fingerprint. Non-secret settings
that affect visible output SHOULD enter one of the two.

### 17.5 Input and coverage fingerprints

```text
inputFingerprint = SHA256(
  "fkst-evolution-input-v2" ||
  productRelevantFingerprint ||
  generatorPinnedFingerprint ||
  normalizedRelevantConfiguration
)

coverageState = ( coverageFingerprint, observedHead )
```

`inputFingerprint` decides whether a cycle runs and whether a sync PR is stale.
`coverageState` records what range was observed.

**How `coverageState` advances.** It has no independent write path, by design.
The manifest advances only when a sync PR merges (section 16.4), so a commit
that changes only coverage leaves `previousCoveredHead` behind the branch head
until the next product-relevant cycle. That cycle then walks the whole range
from `previousCoveredHead` to the current head (section 15.5) and records every
commit in it, product-relevant or not.

The lag is therefore bounded by the next product-relevant change, not
unbounded, and it MUST NOT be treated as non-convergence — see section 17.7
condition 5. A repository that wants coverage flushed without waiting can bump
`generatorEpoch` or request a full rebuild.

The consequence is the intended one: a commit that touches only comments,
tests, CI configuration, or unrelated tooling advances `coverageState`,
appears in provenance and in the covered range of section 15.5, and does **not**
open a sync issue. A commit that touches the product surface changes
`inputFingerprint` and does.

The exact canonical serialization MUST be documented and covered by test
vectors before implementation.

### 17.6 Output fingerprint

The output fingerprint covers everything Evolution wrote:

- every file under `.fkst/evolution/` **except** `config.yaml`, `intent/**`,
  and `manifest.json`;
- all referenced Release asset content hashes; and
- a canonical projection of `manifest.json`.

`manifest.json` is excluded from repository *file* hashing to avoid a circular
hash, because the file contains the fingerprint being computed. A canonical
projection of it MUST nevertheless be included: every field except
`outputFingerprint` itself, serialized with sorted keys and no insignificant
whitespace.

This is a **MUST**, not the previous draft's MAY. Under the MAY, the
`verification` and `artifacts` sections could sit outside the hash while
section 17.7 conditions 3 and 4 read presence and verification status out of
them — so editing those strings in the file changed the answer to "is this
converged?" without changing any fingerprint. The projection closes that.

### 17.7 Convergence decision

The repository is converged only when all of the following hold. Conditions 3
and 4 MUST be evaluated by re-deriving from repository and GitHub state, never
by reading a status field out of the manifest alone.

1. the current input fingerprint equals the committed manifest input
   fingerprint;
2. the current output fingerprint equals the committed manifest output
   fingerprint;
3. every REQUIRED artifact is present, and for each one the manifest's
   `contentHash` matches a freshly computed hash of the blob at its
   `repositoryPath`, or of the referenced Release asset;
4. every REQUIRED verification entry is corroborated per section 17.7.1;
5. no newer **product-relevant** change remains uncovered; and
6. no open canonical sync PR in the artifact repository represents a different
   current input.

Condition 5 deliberately says product-relevant, not authoritative. Under the
section 17.5 split, `coverageState` advances only when a cycle merges, so after
any test-only or CI-only commit the manifest's `previousCoveredHead` legitimately
lags the branch head. Testing condition 5 against *all* authoritative input would
make every such commit a convergence failure, which would admit a cycle and
reproduce exactly the behavior the split exists to remove. Coverage lag is not a
convergence failure; it is reconciled in batch by the next product-relevant
cycle, which walks the full range from `previousCoveredHead` (section 15.5).

A mismatch in condition 3 is managed-output drift and MUST be handled under
section 17.8. Because the mismatch means either the file or the manifest was
edited outside the Evolution lane, it is resolved as `block` regardless of the
repository's configured drift policy — a `contentHash` disagreeing with its own
bytes is an integrity failure, not the ordinary manual-edit case that `repair`
and `adopt` address.

#### 17.7.1 Corroborating verification

"Corroborated" means: the controller re-fetches the check run that recorded the
verification result and independently confirms it. It is not a status string in
the manifest, and it is not a claim by the generation sandbox.

The manifest's `verification` block MUST therefore record, for each required
check, the identity of a GitHub check run:

```json
"verification": {
  "status": "passed",
  "verifiedAt": "2026-07-24T12:00:00Z",
  "checks": [
    {
      "id": "jny_4c81",
      "status": "passed",
      "evidence": ".fkst/evolution/journeys/csv-import.spec.ts",
      "checkRun": {
        "repository": "owner/project",
        "headSha": "<sync PR head sha at verification time>",
        "id": 1234567890,
        "name": "fkst-evolution/journey.jny_4c81"
      }
    }
  ]
}
```

Corroboration succeeds only when all of the following are true:

1. the referenced check run still exists and is retrievable from GitHub;
2. its `app.id` equals the configured FKST App — a check run published by any
   other actor is not evidence;
3. its `conclusion` is `success`; and
4. its `output` records an input fingerprint equal to the manifest's
   `inputFingerprint`.

**Why the check run is anchored to the pre-merge head, and why that works
after merge.** The check run is published on the sync PR's head commit. After
the PR merges, the trusted branch head is a different SHA — a merge or squash
commit — that carries no such check run. An earlier draft of this section
required corroboration "at the recorded input fingerprint" against the *current*
branch head, which made post-merge convergence unreachable: condition 4 could
never hold again, the repository would never report converged, and section 22.1's
post-merge no-op — the property the entire self-trigger design rests on — would
fail into permanent recursion.

The commit the check run is attached to remains reachable in history after the
merge, so the check run remains retrievable indefinitely by id. Corroboration is
therefore a lookup of durable GitHub state, valid before and after merge, and
requires no post-merge write.

Implementations MUST NOT substitute "a check run exists on the current branch
head" for this rule.

### 17.8 Managed-output drift

If input is unchanged but output differs, a human or tool changed a managed
output outside Evolution. `drift.policy` (section 13.2) MAY choose:

- `repair`: regenerate and propose restoration;
- `adopt`: analyze the manual edit and update the model and manifest through a
  normal sync PR; or
- `block`: request ownership resolution.

The default SHOULD be `block` during bootstrap and `repair` only after owners
have explicitly accepted managed ownership.

## 18. Event and discovery contract

### 18.1 GitHub App subscriptions

Evolution requires these webhook subscriptions in addition to existing FKST
issue handling:

- `push`;
- `pull_request`;
- `repository` for default-branch changes when available; and
- `release` only when release-triggered full rebuild is enabled. Namespaced
  Evolution Releases MUST be excluded.

Installation and installation-repository events remain useful for discovery and
access recovery.

### 18.2 Webhook processing

The webhook route MUST:

1. verify `X-Hub-Signature-256` over the exact raw request bytes;
2. parse only fields needed to derive the affected repository and event class;
3. reject an invalid signature;
4. enqueue a best-effort repository hint;
5. return promptly without waiting for generation; and
6. avoid persisting the payload outside GitHub.

A verified but malformed or unsupported event SHOULD be acknowledged without
creating an unbounded redelivery storm. Correctness comes from subsequent full
resynchronization.

### 18.3 Event actions

| Event                           | Condition                                               | Action                                |
| ------------------------------- | ------------------------------------------------------- | ------------------------------------- |
| `pull_request.opened`           | Base is current default                                 | Reconcile PR preview                  |
| `pull_request.reopened`         | Base is current default                                 | Reconcile PR preview                  |
| `pull_request.synchronize`      | Base is current default                                 | Reconcile new PR head                 |
| `pull_request.ready_for_review` | Base is current default                                 | Reconcile PR preview                  |
| `pull_request.edited`           | Base or relevant metadata changed                       | Re-evaluate eligibility               |
| `pull_request.closed`           | Not merged                                              | Clear active preview status if needed |
| `pull_request.closed`           | Merged                                                  | Nudge canonical reconciliation        |
| `push`                          | Ref is current default                                  | Nudge canonical reconciliation        |
| `push`                          | Other ref                                               | Ignore unless configured              |
| `repository.edited`             | Default branch changed                                  | Re-resolve branch and reconcile       |
| `release.published`             | Full rebuild enabled and release is not Evolution-owned | Reconcile with force-full flag        |
| installation access changed     | Repository affected                                     | Re-discover enrollment and access     |

The merge of a pull request normally produces both a `pull_request.closed`
event and a `push` event. Both MUST converge on the same repository hint and
MUST NOT create two work items.

### 18.4 Startup and periodic full resynchronization

On leader acquisition or process startup, the controller MUST enumerate
accessible installations and repositories, discover enrolled repositories, and
reconcile each one. A periodic full resynchronization MUST repeat this process.

An open Evolution trigger issue SHOULD make an enrolled repository discoverable
through existing FKST registration scanning. The control plane MAY also check
for `config.yaml`, but SHOULD avoid an unbounded per-repository contents scan
when issue registration already proves enrollment.

### 18.5 Sweep cadence

Enrolled repositories with an open trigger, live Evolution sandbox, open sync
issue, or open sync PR SHOULD be swept more frequently than inactive
repositories. All intervals are latency controls, not correctness state.

### 18.6 Debounce and coalescing

A webhook burst MAY be debounced in memory. A restart may lose the timer and
run earlier; this is acceptable. A debounce MUST NOT suppress periodic recovery
or cause a trusted source revision to remain uncovered.

The sync issue creation time or latest bot marker MAY provide a GitHub-native
minimum-age signal when a durable quiet period is required.

## 19. Pull request preview

### 19.1 Purpose

PR preview provides early product-impact feedback without changing canonical
Evolution state. It SHOULD report:

- likely affected capabilities and journeys;
- likely additions, changes, deprecations, or removals;
- artifacts expected to become stale after merge;
- missing migration or user-facing information;
- uncertainty that requires author or owner input; and
- whether the PR attempts to modify an Evolution-managed output directly.

### 19.2 Security boundary

PR preview MUST NOT:

- execute code from the PR under a write-capable repository token;
- receive demo, production, publishing, or LLM-provider secrets beyond the
  minimum separately controlled inference credential;
- load PR-authored browser configuration with privileged credentials;
- publish canonical artifacts;
- mutate the PR head branch; or
- update `.fkst/evolution/manifest.json`.

The preferred design is for the control plane to fetch the PR metadata and diff
with a read-only installation token, provide bounded content to a restricted
analysis sandbox, and post the resulting summary through a separate
controller-owned write operation. The untrusted sandbox never receives the
write token.

#### 19.2.1 The inference credential

The preview sandbox holds exactly one secret: the inference credential needed
to reach the configured LLM endpoint. Earlier drafts described preview as
"read-only and secretless" (section 1, section 25.2, invariant 41.4). Read that
as **holding no GitHub, demo, publication, or artifact-repository credential**,
which is the property the threat model actually depends on. It is not literally
secretless, and an implementation claiming otherwise is describing something it
did not build. Sections 1 and 41.4 now carry the qualified wording.

This carve-out has a consequence worth stating plainly: in a deployment where
the inference credential is a single fleet-wide key, exfiltrating it from one
preview sandbox exposes it for every repository. Deployments SHOULD therefore
mint a per-cycle, rate-limited, separately revocable inference credential, and
MUST NOT hand the preview sandbox the same long-lived key used by privileged
generation.

### 19.3 Durable preview marker

Evolution SHOULD maintain exactly one bot-owned preview comment per pull
request. The comment includes a visible summary and an HTML comment marker:

```html
<!-- fkst-evolution-preview:v1
{"head":"<sha>","base":"<sha>","generator":"sha256:...","status":"current"}
-->
```

On a `synchronize` event, Evolution queries the current PR head. If the marker
already represents that head and generator pinned fingerprint, preview is a no-op.

### 19.4 Preview results are advisory

An unmerged PR is not product truth. Preview findings MUST NOT add planned or
available capabilities to the canonical model. After merge, canonical
reconciliation independently verifies the trusted result.

## 20. Canonical reconciliation

### 20.1 Singleton requirement

For each `(source repository, artifact repository, trusted source branch)`
tuple, there MUST be at most:

- one open canonical sync issue;
- one live canonical Evolution execution;
- one open canonical sync PR; and
- one pending canonical Release-asset set for the same input fingerprint.

The generic FKST behavior of processing independent issues in parallel MUST NOT
be used to create one Evolution issue per commit.

### 20.2 Reconciliation algorithm

Each canonical reconciliation performs these steps:

1. Resolve the source repository's current default branch when configuration
   uses `@default`.
2. Resolve the current trusted branch head.
3. Read and validate Evolution configuration and enrollment.
4. Read the committed manifest, observed model, managed output tree, open sync
   issues, open sync PRs, and relevant Release assets.
5. Resolve all Evolution manifest and package references to immutable commits.
6. Compute the six fingerprints of section 17.1.
7. If converged (section 17.7, re-deriving conditions 3 and 4), repair stale
   labels or comments, close an empty stale sync issue when safe, and stop.
   In a mode that never writes (`disabled`, `observe`, and rollout Phase 1),
   reconciliation ends here: it reports the convergence result and its reason
   and performs no further step. Condition 4 is reported as
   `uncorroborated` when no check run has ever been published, rather than as
   a failure — a repository that has never run a cycle has nothing to
   corroborate.
8. Verify the branch ruleset required by section 25.5.1 and the required-check
   registration of section 21.5.1(4); fail closed if either is absent. This
   step gates *writing*, so it is reached only in `propose`,
   `automerge-managed`, and `release-gated`.
9. If a suppression latch (section 26.2) covers the current input fingerprint,
   report and stop.
10. Otherwise, ensure exactly one coalesced sync issue exists.
11. Ensure no second live execution or incompatible sync PR exists.
12. Launch or continue the Evolution package workflow for the exact observed
    source head, with the sandbox token of section 25.5.
13. Analyze all covered changes and update the product model.
14. Regenerate affected artifacts and perform configured full-surface checks.
15. The sandbox pushes the sync branch under its `contents: write` token. The
    **controller** then opens or updates the one sync PR and creates the draft
    Release and its assets — the sandbox holds neither `pull_requests: write`
    nor Release authority (section 25.5).
16. Re-read the trusted branch and recompute `inputFingerprint` from the Git
    tree (section 21.4).
17. If `inputFingerprint` changed, incorporate the new source head and repeat
    generation in the same canonical lane. A change to `coverageState` alone
    does NOT re-enter generation; it is recorded and the cycle proceeds.

    Regeneration rounds MUST be bounded. An implementation MUST enforce a
    maximum number of rounds and a wall-clock deadline per cycle. On exhausting
    either, the lane MUST stop re-entering generation, mark the sync PR
    `BLOCKED` with reason `source-outpaces-cycle`, and report the condition on
    the sync issue rather than looping. Without a bound, a repository whose
    product-relevant commit interval is shorter than its cycle time regenerates
    forever and never merges — the same livelock the section 17.5 split
    removes for non-product commits, surviving for product ones. Each abandoned
    round also strands a draft Release (section 24.4), which section 24.5 will
    not delete without an explicit policy, so an unbounded loop leaks GitHub
    state as well as compute.
18. Publish the `fkst-evolution/input-current` check run (section 21.5.1)
    reflecting the recomputed comparison and the corroborated verification.
19. When current and verified, request policy-compliant merge.
20. After merge, publish eligible draft Release assets, close the sync issue,
    and let the resulting push perform a final no-op convergence check.

### 20.3 Sync issue protocol

The proposed title is:

```text
[evolution] synchronize <source-repository>@<branch>
```

The issue MUST carry the session's Evolution work label and the correct FKST
creator assignee required by session routing. An App-authored issue MUST follow
existing FKST work-authority rules.

Its body SHOULD contain human-readable scope plus a machine marker:

```html
<!-- fkst-evolution-sync:v1
{"source":"owner/project","branch":"main","desiredHead":"<sha>","generation":7}
-->
```

New source events update the desired marker or leave it for the running worker
to discover by re-reading GitHub. They MUST NOT create another open issue.

### 20.4 Issue lifecycle and race closure

The issue remains open while generation, review, required checks, merge, or
post-merge publication is pending. It MAY close automatically after the sync PR
and required assets are canonical.

Consider this race:

1. the worker reads source head A and decides it is current;
2. source head B is pushed;
3. the webhook sees the still-open issue and creates nothing;
4. the worker closes the issue.

Periodic reconciliation compares the committed manifest with head B and creates
or reopens the next sync issue. Therefore correctness does not depend on atomic
issue closure and branch observation.

### 20.5 State machine

```text
                    DISABLED
                       |  enabled: true
                       v
              BASELINE_REQUIRED
                       |  baseline merged
                       v
   +--------------> PENDING <---------------------+
   |                   |  input fingerprint differs
   |                   v                          |
   |                RUNNING ---------------------->  (generation failed)
   |                   |  sync PR pushed          |
   |                   v                          |
   |                PR_OPEN --------------------->|
   |                   |  checks requested        |
   |                   v                          |
   |               VERIFYING -------------------->|
   |                   |  verified + current      |
   |                   v                          |
   |                 READY  --------------------->|
   |                   |  merge requested         |
   |                   v                          v
   |                MERGING ------------------> BLOCKED
   |                   |  merged                   |
   |                   v                           | condition cleared
   |               CONVERGED                       |
   |                   |  new product-relevant     |
   +-------------------+  input, or generatorEpoch +
                          bump, or full rebuild
```

Rules the diagram encodes:

- `BLOCKED` is reachable from every active state and always has a recovery edge
  back to `PENDING` once its condition clears. It is never terminal.
- `CONVERGED` is a resting state, not a sink: a new product-relevant input, a
  `generatorEpoch` bump, or a forced full rebuild returns it to `PENDING`. This
  is the edge that section 22.1's post-merge no-op depends on.
- `READY` is entered from `VERIFYING`, not through `BLOCKED`. The previous
  draft's diagram drew the only path to `READY` through `BLOCKED`, which no
  implementation should reproduce.

Every durable state is projected from GitHub. The state name itself need not be
stored in a database.

The cycle-level names above are distinct from the artifact-level statuses of
section 16.3 and the freshness projection of section 30.3. A repository in
`VERIFYING` may hold artifacts that are `current`, `stale`, and `failed`
simultaneously; the projection surfaces both dimensions rather than collapsing
them into one badge.

### 20.6 Labels

Implementations MAY use labels such as:

- `fkst-evolution-pending`;
- `fkst-evolution-running`;
- `fkst-evolution-blocked`;
- `fkst-evolution-stale`; and
- `fkst-evolution-complete`.

Labels are projections and durable deduplication latches. Reconciliation MUST
repair them when they disagree with authoritative GitHub state.

## 21. Branch and pull request protocol

### 21.1 Dynamic default branch

Evolution requires a target semantic meaning "the repository's current default
branch." The proposed trigger value is:

```markdown
### Source Branch

@default

### Target Branch

@default
```

Because `@` is not currently accepted as a normal branch name, this is a new
sentinel rather than an ordinary branch. It MUST be resolved at reconciliation
and session-launch time. A repository default-branch rename MUST NOT require a
new Evolution trigger issue.

Two existing behaviors must change to accommodate it. The shared trigger
branch-name validator permits only `[A-Za-z0-9._/-]` and explicitly rejects
`@`, so the sentinel is currently unrepresentable. And the existing session
model defaults `### Target Branch` to an auto-created `fkst-hosted-default`
branch, with work PRs merging into that target rather than into the
repository's default branch — Evolution instead targets the resolved default
branch directly. Both are deliberate departures from current session
semantics and MUST be implemented as an Evolution-specific target resolution
rather than by loosening the generic validator for all sessions.

### 21.2 Sync branch naming

A canonical cycle SHOULD use an owned branch such as:

```text
fkst/evolution/issue-<issue-number>/<input-short-hash>
```

The branch is temporary and MAY be deleted after merge. Evolution MUST NOT
force-push shared branches. If the trusted source advances while the sync PR is
open, Evolution SHOULD merge or otherwise safely incorporate the new trusted
head into its owned branch, regenerate outputs, and push a normal follow-up
commit.

### 21.3 Pull request identity

The PR title SHOULD be:

```text
docs(evolution): synchronize product artifacts through <source-short-sha>
```

The body MUST link the sync issue and contain a machine marker with:

- input fingerprint;
- observed source head;
- generator pinned fingerprint;
- verification status; and
- Release asset set, when applicable.

The marker carries no path set: ownership is a fixed prefix (sections 12.1.1
and 12.3), not per-PR configuration. It is an index key only — section 21.4
forbids deciding freshness from it.

Reconciliation MUST identify the canonical PR by marker and App identity, not by
title alone.

### 21.4 Source-head advancement

Before readiness and again immediately before merge, the system MUST recompute
`inputFingerprint` from the trusted Git tree (section 17.5) and compare it with
the fingerprint recorded in the manifest on the sync branch. If it changed, the
PR is stale and MUST NOT merge under autonomous policy until regenerated.

The comparison MUST NOT be made against the PR body marker. A PR body is
editable by any account with write access to the repository while the REST
payload continues to report the App as `user`, so the marker is an index key,
not evidence — consistent with Appendix A's own rule that marker text alone
never grants authority.

Because `inputFingerprint` covers only `source.productRelevant` (section 17.5),
a commit that lands during generation and touches only tests, comments, or CI
configuration does **not** make the PR stale. It advances `coverageState`, is
recorded in the covered range, and the PR merges. This is what makes
`requireCurrentSource: true` a usable default rather than a livelock: without
the split, a repository whose commit interval is shorter than its cycle time
regenerates forever and never merges.

A repository MAY set `requireCurrentSource: false` to permit merging for
product state A after product state B has landed, with a follow-up cycle.

### 21.5 Safe automatic merge

Existing mergeability-only FKST auto-merge is not sufficient for Evolution.
Evolution autonomous merge MUST be scoped to its canonical PR and MUST require:

- the PR is authored by the configured FKST App identity;
- every changed path lies under `.fkst/evolution/` and is not `config.yaml` or
  under `intent/**` (section 12.1.1);
- the recomputed input fingerprint matches the manifest on the sync branch
  (section 21.4);
- required checks passed, and journey verification is corroborated by a
  controller-published check run (section 21.5.1);
- branch protection and required reviews on the **artifact** repository are
  honored;
- no unapproved deletion escaped the managed-path policy; and
- required Release assets exist with matching hashes.

#### 21.5.1 The merge gate is a check run, not a pre-merge callback

The previous draft recommended GitHub native auto-merge or a merge queue while
simultaneously requiring a fingerprint comparison "immediately before merge"
(section 21.4). Those are not compatible. Native auto-merge has no pre-merge
callback: once armed, GitHub merges when required checks turn green, regardless
of what happened to the base branch in the interim. There is no native
mechanism that satisfies section 21.4 by configuration alone, which is why
open question 40.6 is answered here rather than deferred.

The gate is expressed as an artifact GitHub already honors:

1. The control plane publishes a check run named `fkst-evolution/input-current`
   on the sync PR's head commit.
2. That check run is `success` only while the recomputed input fingerprint
   equals the manifest's, every gate above passes, and verification is
   corroborated per section 17.7.1.
3. On every reconcile, the control plane **updates that same check run on the
   unchanged head**, flipping it to `failure` when the trusted branch's
   product-relevant fingerprint has diverged. Base-branch advancement does not
   change the PR head, so nothing in GitHub re-evaluates the gate on its own;
   the level-triggered reconcile is what re-evaluates it.
4. The artifact repository's ruleset REQUIRES that check. Enrollment MUST verify
   the requirement is present (section 25.5.1); a gate no ruleset requires is
   advisory.

This requires `checks: write`, which open question 40.7 previously asked about
only for reading.

**Which mechanism performs the merge.** Step 3 leaves a race that matters only
for *armed native auto-merge*: between a product-relevant push and the reconcile
that flips the check, GitHub may merge autonomously with the controller entirely
out of the loop. Therefore:

- When `publication.requireCurrentSource: true` (the default), the controller
  MUST perform the merge itself: recompute `inputFingerprint` from the trusted
  tree, then `PUT /repos/{owner}/{repo}/pulls/{n}/merge` with the `sha`
  parameter pinned to the verified head. Native auto-merge MUST NOT be armed in
  this mode, because it would merge without that final check. The residual race
  here is bounded and benign — the controller can at worst merge artifacts for
  product state A microseconds before B lands, which the next cycle corrects.
- When `requireCurrentSource: false`, native auto-merge or a merge queue MAY be
  armed, since the repository has accepted eventual follow-up by definition.

The check run remains required in both modes: it is what blocks a *human* from
merging a stale or unverified sync PR, and what makes the gate visible in the
GitHub UI. Note also that `allow_auto_merge` is off by default on new
repositories and the enabling mutation errors when a PR is already mergeable
with no pending required checks, so native auto-merge is not universally
available even where policy permits it.

An unpinned REST merge based on GitHub's `mergeable` field alone MUST NOT be
used in any mode.

#### 21.5.2 Exclusion from the generic FKST auto-merge

The existing repo-level FKST auto-merge hook merges the App bot's mergeable
open pull requests whenever **any** registered session on the repository opted
in, filtering only by author login and GitHub's `mergeable` field. Section 21.5
requires the Evolution sync PR to be App-authored, so it falls squarely inside
that set.

Left alone, this bypasses every gate in section 21.5: a sync PR whose
verification failed and whose fingerprint is stale would be merged on the next
ordinary sweep, in any `publication.mode` including `propose`, violating
invariants 41.7 and 41.11.

The generic hook MUST therefore skip any pull request whose head branch matches
`fkst/evolution/*` or whose body carries the `fkst-evolution-pr:v1` marker.
This exclusion MUST be covered by a regression test. It is not sufficient for
Appendix D to note that the generic hook "must not be reused"; it must be
actively excluded.

### 21.6 Direct push

Evolution MUST NOT push directly to the trusted source branch. All canonical
changes use a pull request even in the most autonomous mode.

## 22. Self-trigger prevention

### 22.1 Fingerprint-based suppression

Evolution MUST NOT rely only on bot author, commit message, or changed-path
filters to suppress recursion. Those are useful optimizations but can be stale
or spoofed. The authoritative test is fingerprint convergence.

After an Evolution PR merges:

- managed output changed;
- the manifest changed;
- trusted branch head changed;
- authoritative source input did not change; and
- current output matches the manifest.

The next push-triggered reconciliation therefore becomes a no-op.

### 22.2 Generator modifying generator inputs

Evolution configuration, human intent, templates classified as authoritative,
and package references are inputs. Autonomous Evolution MUST NOT modify them in
the same managed-output cycle. A proposed change to these inputs requires a
separate reviewed PR, after which a new cycle regenerates outputs.

### 22.3 Generated workflow code

Generated product-operation skills and workflow code live under
`.fkst/evolution/skills/`, which section 17.3 excludes from both source
fingerprints unconditionally. They are therefore always managed output and can
never be authoritative input to their own cycle. The dual-role hazard the
earlier draft guarded against with a configuration declaration is removed
structurally by the single root; no such declaration exists in the section 13.2
schema, and none is required.

The remaining case is a repository that wants generated workflow code to become
a real input. That requires a human to copy or reference it from outside
`.fkst/evolution/` in a separate reviewed pull request. Once it lives outside
the root it is ordinary authoritative source, is covered by
`source.productRelevant` if the owner selects it, and drives the next cycle
normally. The two-stage sequence is thus preserved, but it is enforced by the
directory boundary rather than by a configuration flag that could be
misdeclared.

### 22.4 Evolution Release events

Publishing a namespaced `fkst-evolution/*` Release is an output of Evolution and
MUST NOT trigger a release-driven full rebuild. Release event classification
MUST distinguish owner product releases from Evolution artifact Releases by
validated tag namespace and App identity. Fingerprint convergence remains the
backstop if an event is misclassified.

## 23. Artifact model

### 23.1 Common artifact fields

Every artifact record MUST include:

- stable artifact ID;
- kind;
- source repository and commit;
- input fingerprint;
- generator pinned fingerprint;
- locale and audience when applicable;
- capability and journey dependencies;
- repository path or GitHub Release asset identity;
- content hash;
- freshness status;
- verification status; and
- creation or last-update time.

### 23.2 Dependency graph

Artifacts SHOULD declare capability, journey, template, and source dependencies.
The dependency graph supports selective regeneration, but a full-surface
consistency check still verifies that supposedly unaffected artifacts do not
contain removed capability references, broken links, or invalid terminology.

### 23.3 Documentation

Documentation artifacts MAY include:

- onboarding and quick-start guides;
- task-oriented user guides;
- conceptual documentation;
- UI and CLI reference pages;
- API examples derived from committed schemas;
- troubleshooting;
- migration guides;
- accessibility guidance; and
- release notes.

Generated documentation MUST distinguish current behavior, experimental
behavior, deprecation, and planned behavior. It MUST NOT expose internal secrets
or private implementation notes.

### 23.4 Product-operation agent skills

An agent skill is not merely reformatted documentation. Each generated skill
SHOULD include:

- supported product versions or source fingerprint;
- purpose and bounded scope;
- required permissions and prerequisites;
- inputs and expected outputs;
- side effects;
- deterministic success verification;
- known failure modes;
- referenced scripts and resources; and
- conformance tests against a trusted product build.

Skill format adapters MAY target Ornn or other supported agent-skill formats.
The canonical product facts and journey semantics remain format-independent.

### 23.5 Executable journeys

An executable journey is both product evidence and capture source. For browser
products, Playwright is RECOMMENDED. Other products MAY use CLI, API contract,
mobile, or desktop automation frameworks.

Journeys MUST use stable synthetic fixtures and SHOULD avoid timing,
localization, or viewport assumptions that make capture nondeterministic.

### 23.6 Screenshots

Screenshots used by documentation or decks SHOULD be captured from the same
passing journey revision used to verify the capability. Each capture SHOULD
record:

- journey and checkpoint ID;
- source commit;
- viewport and device scale factor;
- locale and theme;
- fixture revision;
- browser name and version; and
- content hash.

Small current screenshots MAY be committed. Very large image sets SHOULD use
GitHub Releases or a companion repository according to policy.

### 23.7 Demo video

Demo videos SHOULD be produced from passing executable journeys, not from an
independent manual script. A video pipeline MAY add:

- deterministic cursor movement;
- title and end frames;
- captions;
- callouts;
- audio narration;
- chapter markers; and
- locale-specific tracks.

The raw journey must pass before presentation post-processing. Captions are
REQUIRED when narration or meaningful audio is present. Demo data MUST be
synthetic and safe for public inspection at the artifact's confidentiality
level.

### 23.8 Slide decks

Editable slide source SHOULD be committed in a portable format such as Markdown,
Slidev, Marp, or a source format supported by a configured renderer. Rendered
PDF or PPTX MAY be uploaded as Release assets.

Each deck MUST declare audience and purpose, for example:

- product release update;
- customer onboarding;
- sales demonstration;
- internal roadmap review; or
- investor update.

Evolution MUST NOT reuse a claim or image solely because it appears in an older
deck. Every included element must resolve through the current product model and
artifact manifest.

### 23.9 Localization

Each localized artifact is a distinct artifact revision. Translation MUST occur
after canonical source-language facts are current. Locale-specific screenshots
and videos SHOULD execute the localized product journey rather than replacing
text on a source-language image.

## 24. GitHub Release asset protocol

### 24.1 Scope

GitHub Releases store rendered artifacts that are unsuitable for ordinary Git
history, including MP4, WebM, PDF, and PPTX files. GitHub Actions artifacts are
temporary execution outputs and MUST NOT be referenced as durable artifacts.

### 24.2 Release identity

An Evolution asset set SHOULD use a namespaced immutable tag:

```text
fkst-evolution/<first-16-hex-of-input-fingerprint>
```

The tag targets the trusted source commit represented by the artifacts. A full
fingerprint remains in the Release body and manifest, so the shortened tag is
not the sole identity.

### 24.3 Asset identity

Asset names MUST embed a content-hash prefix of at least 16 hex characters
(64 bits):

```text
csv-import.sha256-abcd1234ef567890.mp4
release-update.sha256-1234abcd5678ef90.pdf
```

An existing asset name MUST NOT be replaced with different bytes. A changed
artifact receives a new content-addressed name.

The width matters because the name is the identity. At 8 hex characters
(32 bits) a collision becomes likely within a few tens of thousands of assets,
and the collision is unrecoverable by design: the name is taken, the bytes
differ, and section 26.2's only remedy is "flag inconsistency." 16 hex
characters removes the failure mode rather than reporting it.

Note that GitHub permits deleting and re-uploading an asset under the same
name, so immutability here is a protocol obligation the platform does not
enforce. Section 17.7 condition 3 re-hashes referenced assets on every
reconcile, which is what actually detects a violation.

### 24.4 Two-phase publication

Large artifacts may need durable GitHub storage before the sync PR merges. The
proposed protocol is:

1. Create a draft namespaced Evolution Release targeting the observed trusted
   source commit.
2. Upload content-addressed assets.
3. Verify GitHub-reported size and locally calculated SHA-256.
4. Reference the draft Release and hashes in the sync PR manifest.
5. Merge the sync PR only after required assets are complete.
6. Publish the Release after the manifest becomes canonical, unless policy
   keeps it private or draft.

An abandoned draft Release is harmless orphaned GitHub state. Periodic
reconciliation MAY identify it for retention cleanup but MUST NOT confuse it
with a canonical manifest reference.

### 24.5 Retention

Retention policy is repository configuration. It SHOULD distinguish:

- named product releases, which are preserved;
- current rolling artifacts;
- superseded intermediate snapshots; and
- abandoned drafts.

Deletion is destructive and MUST require an explicit owner policy. Without one,
Evolution reports retention candidates but does not delete them.

### 24.6 Companion repository

Owners concerned about application-repository tag or Release volume SHOULD use
a companion artifact repository. Release assets in that repository still record
the source repository and trusted source commit.

## 25. Security and trust model

### 25.1 Security objectives

Evolution MUST preserve these security properties:

1. An untrusted pull request cannot obtain a repository write token through
   Evolution.
2. An untrusted pull request cannot obtain demo, production, publication, or
   artifact-repository credentials.
3. Repository content cannot expand the agent's permissions or its write
   boundary. Configuration selects among subtrees inside a fixed root; it
   cannot move the root (sections 12.1.1, 13.3.1).
4. Generated artifacts cannot silently disclose secrets or private demo data.
5. A compromised renderer cannot **merge** its own output, cannot write to the
   trusted source branch, and cannot write outside `.fkst/evolution/` in any
   change that reaches that branch.
6. A forged webhook cannot trigger repository access.
7. A malicious repository cannot use prompt content to override package or
   platform policy.

Objective 5 is deliberately narrower than the previous draft's "cannot write
outside the owned sync branch," which no GitHub App token can deliver: an
installation token has no ref scope (section 25.5). The property actually
guaranteed is a conjunction of three enforced mechanisms — the withheld
`pull_requests: write` (25.5), the required branch ruleset (25.5.1), and the
merge-time prefix check (25.8) — not a token capability. Stating it as a token
capability would have made the objective untestable.

### 25.2 Trust zones

| Zone                       | Trust level                  | Examples                                                  |
| -------------------------- | ---------------------------- | --------------------------------------------------------- |
| Control plane              | Privileged                   | Webhook verification, token minting, issue/PR mutation    |
| Trusted generation sandbox | Executes repository code     | Exact default-branch checkout, synthetic demo credentials |
| PR preview sandbox         | Untrusted and read-only      | Pull request diff analysis                                |
| Source repository content  | Data, not instructions       | Code, README, tests, issue text                           |
| Human intent               | Owner-authoritative data     | Product terminology, protected claims                     |
| Rendered artifact          | Untrusted until verified     | HTML, PDF, video, slide output                            |

#### 25.2.1 Hardening the trusted generation zone

The generation sandbox is the zone that actually executes repository code —
build steps, dependency install hooks, browser automation, media renderers —
automatically on every product-relevant change, with no human filing the work.
Calling it "trusted" describes the provenance of its input, not the safety of
its behavior. It therefore requires a hardening profile at least as concrete as
the untrusted preview zone's (section 25.3):

- `contents: write` and no other GitHub permission (section 25.5);
- synthetic demo credentials only, minted at runtime and destroyed with the
  sandbox (section 25.6);
- no production data, publication credentials, or artifact-repository
  credentials beyond the single scoped token;
- egress restricted to an allowlist covering the GitHub API, the configured
  package sources, and the declared LLM endpoint;
- bounded CPU, memory, disk, and wall-clock, enforced by the runtime;
- kernel-level isolation for the sandbox itself; and
- no path by which sandbox output becomes executable input to the privileged
  control plane — the controller parses structured results with a schema and
  never evaluates returned content.

A merged pull request grants its author code execution in this zone on the next
cycle. Repository owners enabling Evolution SHOULD understand that this raises
the stakes of ordinary PR review, and section 25.5.1's ruleset is what bounds
the consequence.

### 25.3 Pull request threat model

A pull request may deliberately modify:

- package scripts;
- build hooks;
- test configuration;
- browser automation;
- shell scripts;
- media renderer input;
- repository instructions aimed at an agent;
- dependencies; or
- source files crafted to cause data exfiltration.

PR preview SHOULD avoid executing repository code entirely. If a future mode
allows limited execution, it MUST use a sandbox with:

- read-only source;
- no installation write token;
- no demo or production secrets;
- no artifact publication credentials;
- restricted network access;
- bounded CPU, memory, disk, and time;
- controller-mediated result return; and
- no ability to influence the privileged process through executable artifacts.

Such a mode remains OPTIONAL and is disabled by the draft schema.

### 25.4 Prompt injection

Repository files, issue bodies, PR descriptions, code comments, test names, and
product data MUST be treated as untrusted content. Package-level instructions
and platform policy take precedence. The agent MUST NOT obey repository content
that asks it to:

- reveal credentials or hidden instructions;
- change the destination repository;
- widen managed paths;
- disable verification;
- alter merge policy;
- contact an unapproved external service;
- upload data outside GitHub; or
- modify human-owned intent.

Structured parsers SHOULD be used for configuration and schemas. Security
decisions MUST NOT be extracted from free-form generated prose.

### 25.5 Token separation

A GitHub App installation token is scoped by repository and permission only.
**There is no ref scope.** Any token carrying `contents: write` can push every
unprotected branch in the repository and can create and publish Releases. All
credential design below is bounded by that fact, and no wording in this
document may imply otherwise.

The generation sandbox is granted `contents: write` and nothing else:

| Phase                          | Holder     | Permissions                                  |
| ------------------------------ | ---------- | -------------------------------------------- |
| Discovery, contents read       | Controller | `contents: read`, `metadata: read`           |
| PR preview comment             | Controller | `pull_requests: write`                       |
| Generation, sync branch push   | **Sandbox**| `contents: write` **only**                   |
| Sync issue and PR mutation     | Controller | `issues: write`, `pull_requests: write`      |
| Merge gate check run           | Controller | `checks: write`                              |
| Merge                          | Controller | `pull_requests: write`, `contents: write`    |
| Release create, upload, publish| Controller | `contents: write`                            |

Withholding `pull_requests: write` from the sandbox is the load-bearing part.
It is what makes "generated code cannot merge itself" a property rather than a
convention, and it is a **MUST**, not the previous draft's SHOULD.

This matters concretely in the current implementation, where the session token
and the auto-merge token are capability-identical (`contents`, `issues`,
`pull_requests` all `write`) — meaning a session pod can merge its own pull
request today. Evolution MUST NOT reuse that token shape.

#### 25.5.1 Residual reach and the required ruleset

Because `contents: write` is repository-wide, withholding `pull_requests:
write` does not confine the sandbox to its own branch. A compromised or
prompt-injected generator can still push to any unprotected branch.

Enrollment MUST therefore require a branch ruleset, and reconciliation MUST
re-verify it every cycle and fail closed when it is absent or has been widened.

**Which branches.** A ruleset is required on the trusted source branch *and*, when
the artifact repository differs, on the artifact repository's target branch —
that is where the sync branch, the merge, and every generated byte actually
live (section 12.4.1). Protecting only the source branch in a companion setup
leaves the repository that holds all the output entirely unprotected.

**Minimum content.** "A ruleset whose bypass list excludes the App" is satisfied
by an inert ruleset that only blocks force pushes, which would leave ordinary
direct pushes available. The ruleset MUST therefore:

1. block direct pushes to the protected branch, so all changes arrive by pull
   request;
2. block force pushes and branch deletion;
3. require the `fkst-evolution/input-current` check run (section 21.5.1(4)) on
   the artifact repository's target branch; and
4. list no bypass actor that resolves to the Evolution App, including via an
   organization role or team.

Without all four, section 21.6's "Evolution MUST NOT push directly to the
trusted source branch" is a policy the credential does not enforce.

Implementations that require containment stronger than a ruleset provides
SHOULD adopt the stricter variant: the sandbox holds no GitHub token at all and
returns a proposed tree plus provenance over a controller-mediated channel,
with the controller performing every commit and push. That variant costs a
result-transport path for large media and diverges from the standard FKST
session execution model, and is therefore OPTIONAL in this draft.

### 25.6 Demo credentials

Demo credentials MUST be ephemeral, least-privilege, and scoped to synthetic
data. They MUST NOT be committed, embedded in browser storage snapshots, logged
in issue comments, or included in screenshots and video frames.

Authentication state SHOULD be created at runtime and destroyed with the
sandbox. A capture verifier SHOULD inspect visible URLs, console output,
network-error text, and rendered pages for token-like or secret-like material.

### 25.7 Production data prohibition

Public or broadly shared artifacts MUST use synthetic fixtures. Production data
is prohibited by default even when a repository owner has access to it. A future
private-artifact exception requires a separate security design and is outside
this draft.

### 25.8 Output confinement

Confinement is a fixed prefix comparison, not a configured path set. Before
opening or updating a sync PR, the controller MUST verify that every changed
path satisfies section 12.1.1:

```text
changed_path starts with ".fkst/evolution/"
  and changed_path != ".fkst/evolution/config.yaml"
  and changed_path does not start with ".fkst/evolution/intent/"
```

Any other path blocks the merge. Symlink traversal and submodule writes MUST
NOT bypass the check: a symlink is compared by its own path, never by its
target, and a submodule pointer change is a change to the submodule path.

This check is a **merge-time veto, not a write prevention** — section 25.5.1
explains why the credential cannot prevent the write itself. The ruleset
required there is what stops a confined-at-merge path set from being bypassed
by a direct push.

### 25.9 Generated-content safety

Evolution SHOULD check generated artifacts for:

- secrets and high-entropy credential patterns;
- private hostnames and internal identifiers;
- accidental user or production data;
- broken or unsafe links;
- active HTML or script where the destination does not permit it;
- unsupported product claims;
- missing accessibility information; and
- media frames containing notifications, browser chrome secrets, or unrelated
  applications.

### 25.10 Supply-chain integrity

Every package and tool that affects output MUST be pinned or resolved to an
immutable revision and included in the generator pinned fingerprint (section
17.4). Installation
steps SHOULD verify checksums for downloaded binaries. Mutable `latest` tags
MUST NOT be sufficient provenance.

### 25.11 Companion repository authorization

The control plane MUST independently verify App installation and write authority
on the companion repository. Source-repository configuration alone does not
grant cross-repository authority. An unauthorized or transferred companion
repository blocks generation.

### 25.12 Durable diagnostics

Issue and PR diagnostics MUST be bounded and redacted. Evolution MUST NOT commit
raw model transcripts or runtime logs. A concise result summary and artifact
provenance are sufficient for durable recovery. Runtime logs may exist only
under the deployment's separately defined ephemeral operational policy; they
are not Evolution state and correctness MUST NOT depend on them.

## 26. Failure handling and recovery

### 26.1 Recovery principle

Every recoverable failure is handled by re-reading GitHub and attempting to
converge again. A durable private retry record is unnecessary. Failures MUST be
visible in GitHub when owner action is required.

### 26.2 Failure matrix

| Failure                                          | Required behavior                                                                         |
| ------------------------------------------------ | ----------------------------------------------------------------------------------------- |
| Webhook is missed                                | Periodic full resync discovers fingerprint drift                                          |
| Webhook is duplicated                            | Repo hint deduplication and convergence make it harmless                                  |
| Events arrive out of order                       | Current GitHub branch/PR state overrides payload order                                    |
| Control plane restarts                           | Startup full resync reconstructs pending work                                             |
| In-memory queue is full                          | Drop hint, log bounded warning, recover on sweep/resync                                   |
| Multiple replicas receive event                  | Leader dispatch plus GitHub singleton checks prevent duplicate canonical work             |
| GitHub issue search lags                         | Recheck enrolled repos and directly list known labels on sweep                            |
| Source branch advances during run                | Incorporate new head and regenerate in same lane                                          |
| Source branch advances just before issue closure | Next push or periodic resync opens/reopens work                                           |
| Default branch is renamed                        | Resolve `@default`, update PR target if possible, otherwise replace blocked PR safely     |
| Default history is force-pushed                  | Detect non-ancestor relation and perform full comparison                                  |
| Configuration is invalid                         | Fail closed and comment on the trigger or sync issue                                      |
| Manifest is absent                               | Create baseline cycle                                                                     |
| Manifest is malformed                            | Block autonomous overwrite and propose reviewed repair                                    |
| Managed output was edited manually               | Apply configured repair, adopt, or block policy                                           |
| Sync branch conflicts                            | Agent resolves only managed paths; source conflicts block and request separate work       |
| Required check fails                             | Keep PR open, report failure, retry after relevant change                                 |
| Journey is flaky                                 | Mark artifact unverified; do not publish as current                                       |
| Screenshot or video is blank                     | Fail capture verification and retain previous current artifact                            |
| Release upload partially fails                   | Keep Release draft, retry missing content-addressed assets                                |
| Release exists with wrong bytes                  | Never replace; create correct content-addressed asset and flag inconsistency              |
| Companion repository loses access                | Block without falling back to another destination                                         |
| Package ref becomes unreachable                  | Keep prior canonical artifacts and report generator resolution failure                    |
| GitHub rate limit is reached                     | Respect reset/retry hints and rely on later reconciliation                                |
| GitHub is unavailable                            | Make no local durability claim; retry after recovery                                      |
| Owner closes sync PR without merge               | Leave manifest unchanged; apply `publication.onOwnerClose` (section 26.2.1)                |
| Trigger issue is closed                          | Retire Evolution runtime; committed artifacts remain historical state                     |

#### 26.2.1 The owner's brake

Closing the sync PR without merging is the owner's primary way to stop a cycle
producing bad output. It MUST therefore have a defined effect. The previous
draft deferred to a "configured suppression/retry policy" that no schema
defined, which in practice means the level-triggered reconciler reopens the
same PR immediately and forever, leaving `enabled: false` as the only brake.

With `publication.onOwnerClose: "suppress-until-input-changes"` (the default):

1. The control plane records a suppression latch as a durable label
   (`publication.suppressionLabel`) on the sync issue, together with the
   suppressed `inputFingerprint` in the issue's machine marker
   (`suppressedInput`, Appendix A.2).
2. While the latch is present and the current `inputFingerprint` equals the
   suppressed one, reconciliation reports `BLOCKED` with the reason and creates
   no new sync PR.
3. Any change to `inputFingerprint`, a `generatorEpoch` bump, or removal of the
   label clears the latch automatically.

**The latch must survive issue closure.** Section 20.4 permits the sync issue to
close automatically, and section 20.2 step 7 permits closing an empty stale sync
issue. If the latch lived only on an open issue, the reconciler would erase the
owner's brake on the next sweep and immediately reopen the loop it was meant to
stop. Therefore, while a suppression latch is set:

- the sync issue MUST NOT be auto-closed, and MUST NOT be treated as "empty
  stale"; and
- discovery MUST look for the suppression label on **closed as well as open**
  sync issues, so that a manually closed issue still suppresses.

An owner clearing the label, or any input change, is the only way to resume.

Suppression is scoped to one input, not to the repository: it stops the loop
the owner objected to without disabling Evolution. `onOwnerClose: "none"`
restores unconditional retry.

### 26.3 Partial success

Evolution MUST NOT merge a manifest that marks all outputs current when a
required artifact failed. Policy MAY allow a partial PR when artifact classes
are independent, but failed artifacts MUST retain their previous canonical
revision or be marked stale/blocked truthfully.

### 26.4 Retry classification

Failures SHOULD be classified as:

- transient, such as GitHub timeout or rate limiting;
- source-dependent, such as a failing journey;
- configuration-dependent, such as an empty `source.productRelevant`;
- authorization-dependent, such as missing companion access;
- review-dependent, such as a protected-fact contradiction; or
- terminal for the current input, such as an unsupported schema.

The issue comment SHOULD state the class, affected artifact IDs, last attempted
source fingerprint, and recovery condition.

### 26.5 Preserving the last known good state

Failure to generate a new artifact MUST NOT delete or overwrite the last known
good canonical artifact. The manifest may mark it stale, but its historical
bytes and provenance remain available until explicit retention policy removes
them.

### 26.6 Orphan reconciliation

Periodic reconciliation SHOULD detect:

- an open sync issue with no live runtime and no PR;
- an open sync PR with no matching issue;
- a draft Release with no matching open PR or canonical manifest;
- a manifest that references a missing Release asset;
- multiple candidate sync issues or PRs; and
- a live runtime for a retired trigger.

It MUST choose a canonical resource using exact machine markers and GitHub
identity, report ambiguity, and avoid deleting user-authored resources.

## 27. Autonomy policy

### 27.1 Modes

Evolution SHOULD support these repository-selected modes:

| Mode                | PR preview | Canonical generation                         | Merge behavior                    |
| ------------------- | ---------- | -------------------------------------------- | --------------------------------- |
| `disabled`          | Off        | Off                                          | None                              |
| `observe`           | Automatic  | Drift report only                            | None                              |
| `propose`           | Automatic  | Automatic sync PR                            | Human merge                       |
| `automerge-managed` | Automatic  | Automatic sync PR                            | Auto-merge after all safety gates |
| `release-gated`     | Automatic  | Continuous docs/model; full media on release | Policy-dependent                  |

Bootstrap SHOULD begin in `propose`. After the baseline model, managed paths,
journeys, and merge checks are approved, owners MAY select
`automerge-managed`.

### 27.2 Autonomous responsibilities

In `automerge-managed`, Evolution SHOULD autonomously:

- detect current trusted changes;
- coalesce work;
- maintain capabilities and journeys;
- update all affected managed artifacts;
- add missing managed journey coverage when safe;
- capture synthetic screenshots and videos;
- render configured decks;
- run cross-artifact verification;
- update the sync PR until it matches the latest trusted source;
- request safe auto-merge; and
- recover from transient failures.

### 27.3 Human review boundaries

Human review remains REQUIRED for:

- changes to product intent or protected facts;
- enabling a managed output class for the first time (section 13.4);
- a new artifact or companion repository destination;
- publication policy changes;
- any change record containing `capabilities.removed` or `journeys.removed` —
  a false removal is a user-visible product claim that generated release notes
  and the section 30.2 timeline will publish (section 14.2.2);
- claims marked regulated, legal, financial, medical, or security-sensitive;
- destructive retention changes;
- source-code changes outside managed output; and
- any condition explicitly configured with `requiresHumanReview`.

### 27.4 Source refactoring findings

When Evolution determines that application source should change, it SHOULD
create a separate issue with evidence and acceptance criteria. It MUST NOT add
that source refactor to the artifact sync PR.

This separation prevents the observer from changing the behavior it is
simultaneously using as evidence.

### 27.5 Uncertainty

Uncertainty MUST be represented explicitly. The system MAY use statuses such as
`unknown`, `unverified`, or `needs-owner-input`. It MUST NOT fill missing product
intent with fabricated certainty merely to keep a run autonomous.

## 28. Package composition

### 28.1 Proposed manifest

An FKST manifest such as `product-evolution.json` SHOULD compose focused
packages rather than one monolith.

| Package role             | Responsibility                                                       |
| ------------------------ | -------------------------------------------------------------------- |
| Evolution observer       | Diff analysis, commit coverage, capability and journey impact        |
| Product cartographer     | Maintain observed model and semantic change records                  |
| Documentation maintainer | Generate and validate user-facing documentation                      |
| Skill builder            | Generate format adapters and run skill conformance tests             |
| Demo producer            | Prepare fixtures, run journeys, capture screenshots and video        |
| Narrative producer       | Generate release narratives and slide sources                        |
| Artifact renderer        | Render PDF, PPTX, video, captions, and other binaries                |
| Evolution verifier       | Enforce ownership, provenance, links, claims, media, and consistency |

Owners MAY omit producer roles for artifact classes they do not need. The
observer, product cartographer, and verifier are REQUIRED. The verifier MUST
understand which producer roles were omitted so their artifact classes are not
reported as missing.

### 28.2 Work routing

The manifest SHOULD declare a dedicated work label such as `fkst-evolution`.
Only the singleton sync issue uses that label. Product source-refactor findings
use a different development workflow label.

### 28.3 Singleton execution primitive

The preferred package/runtime contract is a level-triggered singleton queue
mode, conceptually:

```toml
[github]
work_labels = ["fkst-evolution"]
queue_mode = "singleton-level"
max_in_flight = 1
```

The exact upstream FKST package or engine syntax is outside this hosted draft,
but the behavior is required. If the current engine cannot update one running
work item when desired head advances, the package MUST re-read the trusted head
before completion and the control plane periodic reconcile MUST schedule the
next cycle after closure.

### 28.4 Generator immutability

Trigger and manifest package references may be authored with a branch or tag,
but each run MUST resolve them to exact commits and record those commits in the
generator pinned fingerprint and manifest.

## 29. Required component changes

### 29.1 fkst-hosted control plane

The hosted control plane requires:

- `push`, `pull_request`, optional `repository`, and optional `release` webhook
  classification. The webhook router currently dispatches only `installation`,
  `installation_repositories`, and `issues`; the other events are ignored;
- a GitHub App **permission** change adding `checks: write` (needed to publish
  the section 21.5.1 merge gate, not merely to read check results) and a
  **subscription** change adding Push, Pull request, Repository, and Release.
  These have different rollout costs: added subscriptions take effect
  immediately, whereas an added permission places every existing installation
  in a pending state until an account owner approves it. Enrollment MUST treat
  an installation that has not accepted `checks: write` as not-yet-enrollable
  rather than silently degrading the merge gate;
- branch-name validation that accepts the `@default` sentinel. The current
  validator permits only `[A-Za-z0-9._/-]` and explicitly rejects `@`, so the
  sentinel of section 21.1 cannot be expressed until it is extended;
- current PR base, head, head repository, draft state, merge state, and marker
  comment access;
- an Evolution enrollment and state projector;
- startup, sweep, and full-resync integration;
- one-sync-issue and one-sync-PR enforcement;
- issue update/reopen support;
- dynamic `@default` branch resolution;
- separate PR-preview, generation-sandbox, and controller token paths per the
  section 25.5 table, including a sandbox token that carries `contents: write`
  without `pull_requests: write`;
- branch-ruleset verification per section 25.5.1;
- check-run publication for the section 21.5.1 merge gate, and a `sha`-pinned
  merge fallback;
- an explicit exclusion of Evolution sync PRs from the generic repo-level
  auto-merge hook, with a regression test (section 21.5.2);
- GitHub Release and asset primitives;
- GitHub-native dashboard projection; and
- tests proving no database or external artifact dependency.

### 29.2 fkst packages

The packages repository requires the package roles and composed manifest
described above, plus schemas and validation scripts shared across them.

### 29.3 fkst substrate

The engine may require:

- singleton level-triggered queue semantics;
- explicit maximum in-flight enforcement for Evolution;
- a way for a running workflow to observe desired-head advancement; and
- phase-specific credential boundaries.

If these behaviors can be implemented entirely in packages and hosted
reconciliation without weakening safety, no engine change is required.

### 29.4 Ornn and skill adapters

Product-operation skill generation SHOULD integrate through the current Ornn
contract when Ornn is the selected output. The product model remains canonical;
Ornn-specific layout is a renderer concern. Exact integration MUST be validated
against Ornn's current contract during implementation.

## 30. Stateless dashboard and API projection

### 30.1 No dashboard database

The dashboard MUST NOT persist an Evolution index. It derives repository state
on demand from:

- `.fkst/evolution/config.yaml`;
- `.fkst/evolution/manifest.json`;
- capability, journey, and change files;
- current branch and pull request metadata;
- sync issue labels and comments; and
- referenced GitHub Releases.

In-memory ETag, token, and parsed-manifest caches MAY reduce latency. Cache loss
must only affect performance.

### 30.2 Proposed repository view

A repository-level Evolution workspace SHOULD expose:

- current convergence state;
- current trusted source revision and last covered revision;
- capability and journey map;
- product-change timeline, rendering rename, merge, and split relations as
  such rather than as removals plus additions (section 14.2.2);
- artifact freshness matrix;
- current sync issue and PR;
- blocked or failed verification;
- configured autonomy and publication policy; and
- links to repository files and Release assets.

Evolution is repository-scoped, not session-scoped. Session details may link to
the Evolution cycle that produced an artifact, but artifacts outlive any one
sandbox.

### 30.3 Freshness projection

The UI SHOULD distinguish:

- current and verified;
- current but unverified;
- stale;
- generating;
- awaiting checks;
- awaiting review;
- blocked;
- failed; and
- unavailable.

Status MUST not be inferred from color alone and MUST not display unknown data
as zero or healthy.

### 30.4 API behavior

Any public API SHOULD return a projection timestamp, source GitHub revision,
and partial-data indicators. If GitHub is unavailable or rate-limited, it SHOULD
return an explicit unavailable or incomplete state rather than stale data
presented as current.

## 31. Observability and operator behavior

### 31.1 Durable user-visible reporting

The sync issue SHOULD receive bounded milestone comments for:

- cycle accepted;
- source range being processed;
- sync PR opened or updated;
- verification blocked or failed;
- source advanced and cycle is regenerating;
- merge requested; and
- cycle completed.

One status comment SHOULD be updated where practical to avoid unbounded issue
noise. Durable labels provide compact lifecycle latches.

### 31.2 Runtime metrics

The service MAY expose ephemeral metrics such as:

- reconcile hints received and dropped;
- repositories reconciled;
- converged no-ops;
- preview analyses;
- canonical cycles launched;
- cycle duration;
- generation failures by class;
- stale-head rebuilds;
- artifact counts and bytes; and
- GitHub rate-limit delay.

Metrics are operational observations, not durable Evolution state.

### 31.3 Logging

Logs MUST redact tokens, credentials, private data, prompts containing secrets,
and signed asset URLs. A process restart may lose logs without affecting
correctness. All owner-actionable recovery information must also appear in
GitHub.

## 32. Performance and cost control

### 32.1 Coalescing

Webhook bursts for one repository SHOULD collapse into one reconciliation. A
running canonical cycle SHOULD incorporate a newer trusted head when practical
instead of starting a competing run.

### 32.2 Change-impact analysis

Evolution SHOULD maintain artifact dependencies so expensive work is selective.
Examples:

- an internal backend refactor with unchanged API and journeys may require only
  a consistency check;
- a UI layout change may refresh screenshots and decks without rewriting API
  documentation;
- a terminology change may update docs, skills, captions, and slides; and
- a journey behavior change may regenerate every artifact derived from that
  journey.

### 32.3 Full rebuilds

A full rebuild SHOULD occur when:

- the owner requests it;
- `config.yaml`'s `generatorEpoch` is incremented;
- a configured product release is published;
- `generatorPinnedFingerprint` changes — that is, the repository's own resolved
  package commits, prompts, templates, or schema versions moved;
- schema migration requires it;
- previous manifest ancestry is lost; or
- dependency integrity cannot establish selective safety.

A change to `generatorEnvFingerprint` alone (engine version, model identifier,
renderer versions — section 17.4) MUST NOT trigger a full rebuild. It is
recorded as provenance. The previous draft's condition "generator fingerprint
changes incompatibly" never defined "incompatibly" and, applied to the
environment fingerprint, would have made every operator model roll a
fleet-wide regeneration event.

When an operator does intend a fleet-wide regeneration, it MUST be executed
under a rollout budget bounding how many repositories may enter a cycle per
interval, so that section 18.4's startup resync cannot synchronize the entire
fleet's regeneration with a deploy.

### 32.4 Media generation

Video rendering is expensive and SHOULD occur only when its journey, product UI,
locale, template, narration, or renderer inputs changed, or during a forced full
rebuild. Detection still occurs after every source change.

### 32.5 GitHub API use

Implementations SHOULD:

- use conditional requests and ETags in memory;
- paginate all lists;
- avoid fetching blobs when tree identity proves they are unchanged. Section
  17.2 hashes blob **content**, but a Git tree entry already carries a
  content-addressed blob object id, so an implementation MAY substitute the
  recorded blob object id for a re-fetched content hash whenever the tree
  entry is unchanged, and MUST document that substitution in its test vectors;
- prefer one recursive tree read per revision over per-path contents calls, and
  MUST treat a truncated recursive tree response as a failure rather than
  hashing a partial tree — a partial tree produces a stable but wrong
  fingerprint, which presents as false convergence;
- cache installation resolution briefly;
- respect primary and secondary rate limits, including the separate and much
  tighter limit on the issue-search endpoint used for enrollment discovery; and
- distribute full-resync work with bounded concurrency.

No optimization may replace periodic correctness checks with a durable private
cache.

## 33. Schema and compatibility policy

### 33.1 Versioning

Every Evolution structured file MUST contain `schemaVersion`. Readers MUST fail
closed on a newer unsupported version. Writers MUST write only versions they
fully understand.

### 33.2 Migration

Schema migration occurs through a normal sync PR. A migration MUST preserve
semantic history and artifact provenance. Destructive field removal requires an
explicit migration path or archived source representation.

### 33.3 Forward compatibility

Unknown status or artifact kinds MUST be surfaced as unknown, not coerced to a
known safe state. Extension fields require a namespaced extension mechanism in
a future schema revision.

### 33.4 Existing `.fkst/packages/`

Evolution MUST preserve the existing `.fkst/packages/` role, and treats it as
neither authoritative input nor managed output. It is unconditionally excluded
from both source fingerprints (section 17.3) and unwritable by Evolution
(section 12.1.1 rule 3).

Earlier drafts required configuration to classify workflow files under that
path as source or output. That classification is no longer expressible — the
section 13.2 schema has no path fields — and is no longer needed: the
recursion it guarded against is prevented structurally, because Evolution
cannot write there and changes there cannot enter a fingerprint.

A repository that wants its `.fkst/packages/` catalog to influence Evolution
does so through the FKST manifest and package references, which are resolved to
immutable commits and enter `generatorPinnedFingerprint` (section 17.4).

## 34. Adoption, disablement, and removal

### 34.1 Baseline adoption

Initial adoption SHOULD proceed as follows:

1. Create a draft Evolution configuration and human-intent template.
2. Open or seed the Evolution trigger issue.
3. Run a read-only baseline inventory.
4. Open a baseline PR containing the observed model and a small representative
   artifact set, all under `.fkst/evolution/`.
5. Have owners approve product intent, capability identity, the
   `source.productRelevant` set, and merge policy. Path ownership is not
   negotiable and is not part of this review (section 12.1.1).
6. Run executable journey verification.
7. Merge the baseline.
8. Optionally enable autonomous managed-output merging.

### 34.2 Disablement

Setting `enabled: false` or closing the Evolution trigger retires runtime
activity. Existing files and Release assets remain ordinary repository history.
Disablement MUST NOT delete artifacts.

### 34.3 Removal

Removing Evolution-managed files, tags, Releases, or a companion repository is
a separate owner-authorized cleanup operation. The system SHOULD produce an
inventory and recovery implications before deletion.

### 34.4 Re-enrollment

Re-enrollment reads the existing manifest and verifies it against current
source. It does not assume artifacts remain current merely because the previous
trigger was cleanly retired.

## 35. Test strategy

### 35.1 Unit tests

Unit tests MUST cover:

- configuration parsing, and rejection of an explicit attempt to re-include
  `.fkst/evolution/` or to give a managed output a path;
- branch sentinel resolution;
- canonical path matching;
- product-relevant, coverage, generator-pinned, generator-env, input, and
  output fingerprint test vectors;
- manifest parsing and corruption handling;
- the canonical manifest projection that enters the output fingerprint
  (section 17.6), including that editing `verification` changes it;
- change-range ancestry behavior;
- machine-marker parsing, including that a marker is never accepted as
  authority for a freshness decision (section 21.4);
- event classification;
- singleton selection;
- self-trigger suppression;
- artifact status projection;
- retention candidate classification;
- the single-root prefix check, including the `config.yaml` and `intent/**`
  carve-outs and symlink and submodule cases (sections 12.1.1, 25.8);
- capability identity: rename preserves the identifier, and merge and split
  produce relations rather than remove-plus-add (sections 14.2.1, 14.2.2);
- suppression-latch set, match, and auto-clear (section 26.2.1); and
- secret enforcement.

### 35.2 Property tests

Property tests SHOULD establish:

- file enumeration order does not affect fingerprints;
- boundary encoding prevents ambiguous hash inputs;
- duplicate and reordered event sequences converge to the same plan;
- reprocessing a converged repository is a no-op;
- generated-only commits do not change input fingerprint;
- product-relevant changes always change `inputFingerprint`, and
  non-product-relevant changes never do — the two sets are disjoint by
  construction (section 17.5);
- a change to `generatorEnvFingerprint` alone never changes `inputFingerprint`
  (section 17.4);
- no path outside `.fkst/evolution/` can pass output validation, for any
  configuration whatsoever — this MUST be tested against adversarial
  configuration, not only valid configuration; and
- a truncated recursive tree read fails rather than producing a fingerprint
  (section 32.5).

### 35.3 GitHub integration tests

A fake or disposable GitHub repository test harness SHOULD cover:

- installation discovery;
- push and PR webhook verification;
- missed webhook recovery by full resync;
- one issue/PR under concurrent hints;
- PR head advancement;
- default-branch rename;
- force-pushed history;
- issue search lag;
- safe merge gating;
- draft Release upload and publication;
- companion repository access loss; and
- branch protection behavior.

### 35.4 Security tests

Security tests MUST include malicious PRs attempting to:

- read environment secrets;
- execute modified test hooks;
- rewrite configuration;
- escape managed paths through symlinks;
- instruct the agent to upload data externally;
- inject misleading machine markers;
- create an unauthorized companion destination; and
- hide tokens in screenshots, video, or generated HTML.

### 35.5 End-to-end artifact tests

At least one representative product fixture SHOULD prove:

1. a default-branch product change is detected;
2. a capability and journey update is generated;
3. documentation and a product skill are updated;
4. a deterministic screenshot and short video are captured;
5. a slide source and rendered deck are produced;
6. all artifacts reference the same source commit and capability IDs;
7. the sync PR passes checks and merges;
8. the resulting push reconciles to a no-op; and
9. a full control-plane restart still reports the repository converged.

### 35.6 Failure-injection tests

Tests SHOULD kill the controller or sandbox:

- after issue creation;
- during model update;
- after branch push but before PR creation;
- during asset upload;
- after PR readiness but before merge;
- after merge but before issue closure; and
- while a newer source push arrives.

Every case must recover from GitHub state without a database.

## 36. Acceptance criteria

The initial implementation is acceptable only when all of the following are
demonstrated:

### 36.1 Statelessness

- No Evolution database, durable queue, external object store, or Git LFS is
  required.
- Destroying all control-plane process state and sandboxes does not lose
  pending work.
- Startup full resync reconstructs repository status from GitHub.
- Dashboard state can be projected from GitHub resources alone.

### 36.2 Detection and convergence

- Every default-branch push produces a reconcile hint.
- Every relevant PR head update produces a preview reconcile hint.
- A missed webhook is recovered by periodic resync.
- Duplicate and reordered events do not create duplicate canonical work.
- A burst of commits produces at most one open sync issue and one open sync PR.
- Every reachable commit since the previous manifest is considered.
- A merged Evolution PR causes a no-op follow-up rather than recursion.

### 36.3 Trust and permissions

- PR preview cannot access a repository write token or demo credentials.
- Canonical generation runs only from the trusted branch revision.
- Changed paths are confined to `.fkst/evolution/`, and no configuration can
  widen that boundary, demonstrated against adversarial configuration.
- The generation sandbox cannot merge its own pull request, demonstrated by
  attempting the merge API call with the sandbox token and observing refusal.
- A sync PR whose input fingerprint went stale cannot merge, demonstrated
  against an already-armed GitHub auto-merge.
- The generic repo-level FKST auto-merge hook does not merge an Evolution sync
  PR, demonstrated with a session that opted into auto-merge on the same
  repository.
- Product intent cannot auto-merge through managed-output policy.
- Merge honors required checks and branch protection.
- Secret scanning covers text and captured media metadata or visible output.

### 36.4 Artifact correctness

- Capabilities and journeys have stable opaque IDs and evidence, and a title
  rename or a full rebuild preserves them.
- Docs, skills, screenshots, videos, and slides share the same product model.
- Every artifact records source, input, and generator-pinned fingerprints.
- Failed required verification cannot be represented as current and verified,
  demonstrated by hand-editing `manifest.json` to claim success and observing
  that the repository does NOT report converged.
- The previous good artifact remains available when a new generation fails.
- Large binaries are durable GitHub Release assets with content hashes, and a
  manifest referencing a missing or rewritten asset does not report converged.

### 36.5 Mechanisms introduced by revision 2

Each of these is load-bearing for an invariant and MUST be demonstrated, not
merely implemented:

- The generation sandbox's token carries `contents: write` and not
  `pull_requests: write` (invariant 41.18).
- The required branch ruleset is verified every cycle on both the source and
  artifact target branches, and its absence fails closed (section 25.5.1).
- A repository in `observe` mode, and rollout Phase 1, evaluate convergence and
  perform no write at all (section 20.2 step 7).
- A hand-edited `manifest.json` claiming `passed` does not produce convergence,
  because corroboration re-fetches the check run (section 17.7.1).
- A test-only commit advances `coverageState`, admits no cycle, and does not
  invalidate an open sync PR (sections 17.5, 17.7 condition 5, 21.4).
- After a sync PR merges, the repository reports converged — the check run
  referenced by the manifest is still retrievable on the pre-merge head
  (section 17.7.1).
- A capability rename produces a `renamed` relation and preserves its
  identifier; a merge does not appear as a removal (sections 14.2.2, 15.3).
- Closing a sync PR sets a suppression latch that survives sync-issue closure
  and blocks regeneration for that input only (section 26.2.1).
- A cycle whose source keeps advancing terminates as `BLOCKED` with reason
  `source-outpaces-cycle` rather than regenerating indefinitely (section 20.2
  step 17).

### 36.6 Recovery

- Source advancement during generation is detected before merge.
- A push racing issue closure is eventually processed.
- Default-branch rename is recovered without editing a frozen literal branch.
- Non-ancestor source history triggers a full comparison.
- Partial asset upload is retryable without replacing existing bytes.
- Companion access loss blocks safely.

## 37. Rollout plan

The ordering principle is **riskiest assumption first**. The component whose
being wrong invalidates everything downstream is the convergence oracle, not
the generators — so it ships first, alone, writing nothing.

### Phase 0: specification and schemas

- Review this draft with hosted, substrate, packages, product, and security
  owners.
- Confirm that GitHub issues, comments, labels, PRs, and Releases satisfy the
  repository-only persistence rule.
- Finalize schema canonicalization and fingerprint test vectors.
- Decide ownership of any singleton engine primitive (open question 40.5).

### Phase 1: convergence oracle — zero generation, zero writes

- Configuration parsing and section 13.3 validation, including the section
  12.1.1 write boundary.
- Section 17.2 canonical hashing with published test vectors.
- All six fingerprints of section 17.1.
- Manifest read, parse, validate, and section 17.7 evaluation.
- Startup, sweep, and periodic full resync.
- Output is one line per repository: converged, or not converged with the
  reason. The only permitted write is an optional single sync-issue comment.
- Run in `observe` mode against three to five real repositories for at least a
  week.

This phase answers questions 40.16, 40.17, and 40.18 with measurement rather
than argument: how often a cycle would have fired, what an operator model roll
does to the fleet, and what section 17.8 drift looks like in practice — all at
zero blast radius.

### Phase 2: one artifact class, `propose` only, one pilot repository

- Documentation only. No skills, journeys, media, Releases, or auto-merge.
- Add push and pull_request webhook classification, singleton issue, and
  dynamic `@default` resolution.
- Prove section 22 self-trigger suppression and section 20.2 step 17
  regeneration against real pushes.

### Phase 3: the merge gate as a first-class object

- The `fkst-evolution/input-current` check run (section 21.5.1).
- The `sha`-pinned controller merge fallback.
- Exclusion of sync PRs from the generic auto-merge hook (section 21.5.2),
  with its regression test.
- The branch-ruleset enrollment precondition (section 25.5.1) and the section
  25.5 token split.
- `automerge-managed` only after all of the above, and only on a disposable
  repository.

### Phase 4: PR preview

- Read-only preview, marker comments, and the scoped inference credential of
  section 19.2.1.

Preview is an independent risk surface and is deliberately no longer bundled
with the merge gate; debugging both at once conflates two failure domains.

### Phase 5: media and Releases

- Deterministic screenshot verification.
- Video and slide renderers.
- Draft Release asset protocol and retention projection.
- Synthetic demo environment hardening.

Sequenced after a settled merge gate because the two-phase Release protocol
interacts with regeneration rounds, which strand draft Releases.

### Phase 6: user-facing Evolution workspace

- Repository-level capability, timeline, artifact-health, and cycle views.
- Derive all views live from GitHub.
- Add manual `Reconcile`, `Full rebuild`, and `Prepare release kit` commands
  without creating new durable backend state.

## 38. Worked example

Assume pull request `#412` adds CSV import and targets `main`.

### 38.1 Before merge

1. GitHub sends `pull_request.opened`.
2. The webhook verifies the signature and enqueues a repository hint.
3. PR preview confirms the current head and base.
4. A restricted analyzer reads the diff without repository write credentials.
5. Evolution updates its single PR comment:
   - likely a new capability and journey (identifiers are NOT allocated by
     preview — see step 6 and section 19.4);
   - docs, skill, screenshot, video, and release deck likely affected;
   - upload-size behavior needs evidence.
6. No canonical product model or artifact changes occur.

### 38.2 After merge

1. GitHub sends `pull_request.closed` and `push` hints.
2. Repository reconciliation reads `main` and confirms the merge commit.
3. The commit touched `source.productRelevant` paths, so `inputFingerprint`
   differs from the committed manifest and a cycle is admitted. Had it touched
   only tests or CI configuration, `coverageState` would have advanced and no
   cycle would have opened.
4. Evolution verifies the required branch ruleset (section 25.5.1), ensures one
   sync issue, and starts one trusted generation cycle. The sandbox receives
   `contents: write` and no `pull_requests: write`.
5. The observer processes the commit range and confirms `#412` metadata.
6. The cartographer adds the capability and journey with evidence.
7. The documentation maintainer writes the user guide and limits.
8. The skill builder creates a tested CSV import operation skill.
9. The demo producer provisions synthetic data, executes the journey, captures
   a mapping screenshot, and records a short captioned video.
10. The narrative producer updates the release deck using the new verified
    screenshot.
11. The renderer produces video and PDF; the **controller** creates the draft
    content-addressed GitHub Release and uploads them (section 25.5 — the
    sandbox has no Release authority).
12. The verifier checks paths, claims, links, skill behavior, screenshot pixels,
    video duration and frames, captions, hashes, and source provenance.
13. The sandbox pushes the sync branch; the **controller** opens or updates the
    one sync PR carrying model, change record, manifest, docs, skill, journey,
    screenshot, and slide source, all under `.fkst/evolution/`.
14. The control plane recomputes `inputFingerprint` from the Git tree — not
    from the PR marker — and, finding it unchanged, publishes
    `fkst-evolution/input-current` as `success`.
15. That check is required by the artifact repository's ruleset, so GitHub
    merges the PR once it and the other required checks pass. Where auto-merge
    cannot be armed, the control plane merges with the `sha` parameter pinned
    to the head it verified.
16. The draft Release is published and the issue closes.
17. The generated merge push triggers reconciliation. It touched only
    `.fkst/evolution/`, which is excluded from both source fingerprints, so
    `inputFingerprint` is unchanged and the recomputed output fingerprint now
    matches the manifest. No new work is created.

### 38.3 Later source change during generation

If another **product-relevant** commit reaches `main` during step 9, Evolution
does not open a second sync issue. The control plane flips
`fkst-evolution/input-current` to `failure`, which vetoes any already-armed
auto-merge. The lane incorporates the new trusted head into its owned branch,
analyzes the additional commit, regenerates affected outputs, and updates the
same sync PR.

If the interleaved commit touches only non-product-relevant paths — a test, a
comment, a CI tweak — `inputFingerprint` is unchanged, the check stays
`success`, the PR merges, and the commit is recorded in `coverageState`. This
is the case that made the previous draft livelock on any repository whose
commit interval was shorter than its cycle time.

## 39. Rejected alternatives

### 39.1 Hosted Evolution database

Rejected because it violates the stateless constraint, creates migration and
backup responsibility, and can disagree with GitHub.

### 39.2 Durable webhook event queue

Rejected as a correctness source. It adds storage while still requiring current
state reads because GitHub events are duplicated and reordered. Git history and
current PR state already preserve the information Evolution needs.

### 39.3 One work issue per commit

Rejected because current FKST work items run independently and can produce
conflicting PRs against shared model and artifact paths.

### 39.4 Direct pushes to the default branch

Rejected because they bypass repository review and checks, make recovery less
transparent, and expand the effect of a compromised generator.

### 39.5 Running full production on every PR head

Rejected because contributor code is untrusted and can exfiltrate write tokens
or demo credentials. PR preview remains read-only.

### 39.6 Suppression by bot author or commit message

Rejected as the primary loop guard because it does not prove artifact
convergence and can hide a real authoritative change included in the same
commit. Fingerprints are authoritative.

### 39.7 Storing all media in Git history

Rejected as the default because frequent video and deck binaries permanently
bloat repository history. Small documentation images remain reasonable Git
content; large rendered outputs use GitHub Releases.

### 39.8 GitHub Actions artifacts

Rejected for durable output because they expire and are tied to workflow-run
retention rather than product provenance.

### 39.9 Independent artifact agents with independent product interpretation

Rejected because docs, skills, demos, and decks would drift semantically. All
renderers consume the shared product model.

### 39.10 Rewriting every artifact byte after every commit

Rejected because it creates unnecessary cost, noisy diffs, media churn, and
merge pressure. Evolution inspects the full surface and selectively rewrites
affected views, with explicit full rebuilds when required.

## 40. Open questions

The following decisions remain open for implementation review:

1. Is GitHub repository persistence explicitly defined to include issues,
   comments, labels, PRs, and Releases, or only Git objects? Existing FKST
   already depends on issue resources, so this draft assumes the broader
   definition.
2. Should the initial implementation require a companion repository for video,
   or default to Releases in the source repository?
3. What Release retention defaults are acceptable for high-commit-rate
   repositories?
4. Should one draft Evolution Release be created per input fingerprint or only
   when at least one large artifact changed?
5. Can singleton level-triggered behavior be implemented entirely in packages
   and hosted reconciliation, or is an upstream substrate queue mode required?
   **This one is load-bearing, not deferrable.** It scopes Phases 2-3 (Phase 1
   runs no work items at all), and it
   compounds the admission problem: if a running work item cannot observe head
   advancement, every source advance restarts the cycle from zero. Answer it in
   Phase 0. Note also that CLAUDE.md forbids kernel-engine changes from this
   repository, so a substrate requirement is a cross-repository dependency
   rather than an implementation detail.
6. ~~Which GitHub native merge mechanism best preserves branch protection?~~
   **Answered in section 21.5.1: none of them can.** Native auto-merge has no
   pre-merge callback, so it cannot satisfy section 21.4. The gate is an
   Evolution-owned required check run plus a `sha`-pinned merge fallback. This
   was a design defect, not a mechanism selection.
7. ~~Which permission reads repository checks?~~ **Answered: `checks: read`
   would suffice for reading, but section 21.5.1 requires `checks: write` to
   publish the gate.** The open item is not which permission, but the rollout:
   adding a permission places every existing installation in a pending state
   until an account owner approves it (section 29.1).
8. What is the canonical cross-format product-skill schema before rendering to
   Ornn or another adapter?
9. Which product model fields require explicit owner approval during baseline?
10. Should semantic change records be created for every covered commit or only
    commits classified as product-visible, with coverage retained in manifest?
11. How should private repositories expose rendered media to authorized viewers
    without durable signed URLs in the manifest?
12. Which slide source and editable export formats are required for the first
    version?
13. What deterministic demo-environment contract can be standardized across
    browser, CLI, API, mobile, and desktop products?
14. How should localization review and human translation overrides be modeled?
15. What maximum repository and artifact sizes should trigger companion-repo
    guidance or block generation?
16. What belongs in `source.productRelevant`, and does a defensible default
    exist at all? **Deliberately deferred to Phase 1 measurement rather than
    decided here.** Section 13.2's example is illustrative only and no default
    ships; enrollment requires an explicit declaration. The asymmetry is why:
    a too-broad set is merely expensive and visibly so, while a too-narrow set
    fails silently — the artifact that was never regenerated generates no
    signal, and nobody files a bug for a thing that did not happen. Phase 1's
    read-only oracle reports where it *would* have fired across real
    repositories, converting this from a guess into an observation.
17. Should `generatorEnvFingerprint` ever gate convergence, and if so under
    what fleet-wide rollout budget (section 32.3)?
18. What is the observed rate of section 17.8 managed-output drift on real
    repositories, and does that justify `repair` as a post-bootstrap default?
19. Does the `.fkst/evolution/` single root create adoption friction severe
    enough to warrant a supported publication step (section 12.1.2), or is
    consumer configuration sufficient in practice?

Questions 5, 6, and 7 were load-bearing rather than deferrable. 6 and 7 are now
answered in the body (sections 21.5.1 and 29.1). Question 4 remains genuinely
open: section 24.4 describes the two-phase protocol unconditionally, and
whether a draft Release should be created per input fingerprint or only when a
large artifact changed is a retention-cost trade-off that section 20.2 step 17's
round bound now makes measurable rather than speculative.

## 41. Required invariants

Any implementation claiming conformance to this draft MUST preserve these
invariants:

1. GitHub contains all durable Evolution state.
2. Webhooks are hints; full resync is authoritative recovery.
3. Each `(source repository, artifact repository, trusted source branch)` tuple
   has at most one canonical Evolution lane (section 20.1).
4. PR preview holds no GitHub, demo, publication, or artifact-repository
   credential. It does hold a scoped inference credential (section 19.2.1);
   "secretless" in earlier drafts overstated this.
5. Canonical executable generation uses a trusted source revision.
6. Evolution never directly pushes the trusted branch, enforced by a branch
   ruleset rather than by token scope (section 25.5.1).
7. Autonomous merge is path-scoped, current-head-scoped, and check-gated, with
   the gate expressed as a required check run the control plane owns
   (section 21.5.1).
8. Human product intent is not an autonomous managed output.
9. Input and output fingerprints prevent recursive self-triggering.
10. Every artifact records exact source and generator provenance.
11. Required verification failure cannot appear as current success. This holds
    only because section 17.7 re-derives conditions 3 and 4 from repository
    state and a controller-published check run, rather than reading a status
    field out of the manifest the generator wrote.
12. A failed run preserves the last known good canonical artifact.
13. Large durable binaries remain in GitHub, not external storage.
14. A full restart can reconstruct convergence and pending work from GitHub.
15. Source refactoring and product artifact synchronization remain separate
    work streams.
16. Evolution writes only under `.fkst/evolution/`, never to `config.yaml` or
    `intent/**`, and this boundary is a control-plane prefix comparison that
    repository configuration cannot widen.
17. Cycle admission and merge staleness are decided by the product-relevant
    fingerprint, so a non-product commit neither launches a cycle nor
    invalidates an open one.
18. The generation sandbox never holds `pull_requests: write`, so generated
    output cannot merge itself.

## 42. Draft decision summary

This draft recommends the following initial decisions:

- Use the source repository and its GitHub Releases by default.
- Support a configured companion GitHub repository as an opt-in.
- Store all Evolution state, human intent, and every generated artifact source
  under the single root `.fkst/evolution/`, with large rendered binaries as
  content-addressed Release assets.
- Point consumers at that root through their own configuration (section
  12.1.2); Evolution never copies generated files to conventional paths.
- Make the write boundary a control-plane prefix comparison that repository
  configuration cannot widen.
- Detect both pull request changes and default-branch pushes.
- Treat PR processing as advisory read-only preview.
- Treat trusted-branch processing as canonical generation.
- Coalesce all canonical work into one issue and one PR per repository.
- Resolve the current default branch dynamically through `@default` semantics.
- Compare the six fingerprints of section 17.1 on every reconcile, admitting
  cycles on product-relevant change only.
- Re-derive convergence from repository state and a controller-published check
  run rather than from status fields the generator wrote.
- Gate merges on an Evolution-owned required check run, and perform the merge
  from the controller with a pinned head whenever current source is required.
- Give the generation sandbox `contents: write` and never `pull_requests:
  write`, with a branch ruleset bounding the residual reach.
- Auto-merge only after managed-path, current-source, verification, and branch
  protection gates pass.
- Generate heavy media selectively and store it as content-addressed GitHub
  Release assets.
- Build the first proof around one end-to-end product journey that produces a
  model update, documentation, a tested skill, a screenshot, a short video, and
  a slide from the same evidence.

## Appendix A. Proposed machine markers

Machine markers allow stateless discovery without parsing natural-language
titles. Marker JSON MUST be bounded, schema-versioned, and validated. User text
outside the marker is never interpreted as machine state.

### A.1 PR preview comment

```html
<!-- fkst-evolution-preview:v1
{"source":"owner/project","pr":412,"head":"<sha>","base":"<sha>","generator":"sha256:...","status":"current"}
-->
```

### A.2 Sync issue

```html
<!-- fkst-evolution-sync:v1
{"source":"owner/project","artifactRepo":"owner/project","branch":"main","desiredHead":"<sha>","generation":7,"suppressedInput":null}
-->
```

`suppressedInput` carries the `inputFingerprint` suppressed by the section
26.2.1 owner brake, or `null` when no latch is set.

### A.3 Sync pull request

```html
<!-- fkst-evolution-pr:v1
{"issue":417,"input":"sha256:...","sourceHead":"<sha>","generator":"sha256:...","verification":"passed"}
-->
```

Markers MUST NOT contain credentials, private environment identifiers, signed
URLs, raw prompts, or unbounded artifact lists.

Markers are trusted only when attached to the expected resource, authored by
the configured App identity, and consistent with current GitHub state. Marker
text alone never grants authority or establishes singleton ownership.

## Appendix B. Example artifact freshness matrix

| Capability   | Documentation | Skill               | Screenshots | Video          | Deck    |
| ------------ | ------------- | ------------------- | ----------- | -------------- | ------- |
| CSV import   | Current       | Current             | Current     | Current        | Current |
| Team roles   | Current       | Needs review        | Current     | Not configured | Current |
| Audit export | Draft         | Failed verification | Missing     | Not configured | Draft   |

The matrix is a live projection from the manifest and current GitHub state. It
is not stored in a dashboard database.

## Appendix C. Minimal first proof

The smallest proof that exercises the architecture SHOULD contain:

1. `.fkst/evolution/config.yaml` with an explicit `source.productRelevant` set
   (managed destinations are fixed by schema, not configured);
2. human-authored product intent;
3. one observed capability;
4. one executable journey;
5. one semantic change record;
6. one documentation page;
7. one tested product-operation skill;
8. one screenshot captured from the journey;
9. one short captioned video uploaded to a draft GitHub Release;
10. one editable slide source and rendered PDF;
11. one complete manifest tying them to the same source and generator
    fingerprints;
12. one sync issue and one sync PR;
13. safe merge after checks; and
14. a restart plus reconciliation that proves the result is a no-op.

This proof is intentionally vertical. It validates the central product model
and stateless convergence before broadening artifact formats or dashboard UI.

## Appendix D. Current implementation touchpoints

The following current fkst-hosted areas inform implementation planning. They do
not imply that Evolution behavior already exists:

| Area                                     | Current location                                                    | Evolution relevance                                                         |
| ---------------------------------------- | ------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| Durable-datastore-free application state | `backend/src/state.rs`                                              | Establishes that correctness is not backed by a session database            |
| Signature-verified webhook dispatch      | `backend/src/routes/github_app_webhook/`                            | Add push, PR, repository, and optional release classification               |
| Reconcile queue, sweep, and full resync  | `backend/src/reconcile/loops.rs`                                    | Reuse hint-plus-level-reconciliation recovery pattern                       |
| Active repository projection             | `backend/src/reconcile/mod.rs`                                      | Extend discovery to enrolled Evolution repositories                         |
| Trigger branch configuration             | `backend/src/github_app/templates_assets/fkst-substrate-session.md` | Add dynamic `@default` semantics or an Evolution-specific equivalent        |
| Existing mergeability-only auto-merge    | `backend/src/reconcile/automerge.rs`                                | Must not be reused as Evolution's safety-gated merge policy                 |
| GitHub App permissions and API wrapper   | `backend/src/github_app/`                                           | Add phase-specific PR reads, issue mutation, checks, and Release operations |
| Repo-local workflow catalog              | `backend/src/session_pod/driver.rs`                                 | Preserve `.fkst/packages/`; add `.fkst/evolution/` without collision        |
| Committed outcome projection             | `backend/src/routes/canvas/outcomes.rs`                             | Existing basis for previewing generated text and media in a session view    |
| Repository dashboard                     | `frontend/src/pages/dashboard.tsx`                                  | Potential host for a repository-level live Evolution projection             |

Implementation work MUST continue to respect repository scope boundaries:
hosted user-facing and public-interface work belongs here, package behavior
belongs in `fkst-packages`, and required kernel behavior belongs in
`fkst-substrate`.
