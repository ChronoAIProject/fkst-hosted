---
name: fkst substrate session
about: Start an fkst agent session against this repository.
title: "[session] "
labels: [fkst-substrate-trigger]
---

<!--
Opening this issue starts an fkst session. fkst parses the sections below by
their EXACT `### ` headings. Do NOT rename the headings. Do NOT put secrets,
tokens, or credentials anywhere in this issue — they are supplied only through
your environment and are never read from issue text.
-->

### Session Name

<!-- One line. Lower-case DNS-label: ^[a-z0-9]([a-z0-9-]*[a-z0-9])?$, 1-40 chars. -->
my-first-session

### Packages

<!--
One package reference per line, each as `owner/repo@ref:path/to/package`, pointing
at a PUBLIC repo that has an `fkst.toml` at that path. At least one is required.
-->
ChronoAIProject/fkst-packages@main:packages/example

### Work Label

<!--
One line. The GitHub label whose OPEN issues are this session's work queue. Max 50
characters, no commas. Create work items as separate issues carrying this label
(see the "fkst work item" template).
-->
fkst-work

### Environment

<!--
Optional. One line naming an environment you created via
`PUT /api/v1/users/me/environments/<name>`. Delete this whole section to run with
no environment.
-->

### Auto-merge

<!--
Optional. Set to `true` to have fkst automatically merge THIS session's pull
requests into the default branch as soon as GitHub reports them mergeable. This
BYPASSES review and status checks — use it only on trusted repos. Accepted truthy
values: true / yes / on / enabled / 1 (case-insensitive). Anything else, or
deleting this section, leaves auto-merge OFF (the default).
-->
false
