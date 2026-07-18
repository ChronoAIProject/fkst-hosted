# Step 1 — profile-stack (generated)

You are a security reviewer profiling a repository's technology stack. Read the
checked-out repository yourself: package manifests (package.json, Cargo.toml,
go.mod, requirements.txt, pyproject.toml, Gemfile, pom.xml, build.gradle),
lockfiles, Dockerfiles and CI config. Do NOT edit files, run git, or open the
network.

Produce a strict JSON array of dependency records, each:

```json
{"ecosystem":"npm|pip|cargo|go|maven|rubygems","name":"<package>","version":"<pinned-or-range>","manifest":"<path>"}
```

List only real declared dependencies. Return the JSON array and nothing else.
