# Orbweaver

Ecosystem intelligence and leverage engine for ELCI-group. Full mandate:
`docs/ROADMAP.md` and the original directive.

The core thesis: ecosystem value is not the sum of per-repository value —
the highest-leverage work is usually in the interactions between projects
(`V_ij`, `V_ijk`, `V_feedback`), not inside any one of them. Orbweaver's job
is to find those interactions with evidence, not vibes.

## What's built (Phase I — Foundation)

- **Discovery**: finds candidate repositories under a root directory
  without a hard-coded inventory — a directory counts if it has a `.git`
  or a recognised manifest (`Cargo.toml`, `package.json`, `pyproject.toml`,
  `go.mod`).
- **Ingestion**: deterministic extraction of repo metadata, declared
  dependencies, README presence, and git history (commit count,
  contributors, last-commit time) — no LLM involved (directive section
  4.2: graph first, LLM second).
- **Dependency resolution**: cross-references every declared dependency
  against the other repositories found in the *same* scan, resolving
  `depends_on` edges from path dependencies (strong evidence) and name
  matches (weaker but still deterministic). This is the seed of the
  capability graph in section 9.
- **Evidence**: every extracted fact is stored as an `Evidence` record
  with a source, an extractor, and one of five explicit confidence classes
  (`Observed`, `DeterministicInference`, `ProbabilisticInference`,
  `LlmHypothesis`, `ProposedOpportunity`) — directive section 4.1's
  evidence-before-inference rule, enforced in the type system rather than
  by convention.
- **Snapshots**: each scan is persisted as an immutable, timestamped
  snapshot in a local SQLite store (`~/.local/share/orbweaver/orbweaver.db`).
- **CLI**: `scan`, `status`, `snapshots`, `graph`, `doctor`.

Run it against the actual ELCI estate on this machine:

```bash
cargo run -p orbweaver-cli -- scan --root /home/sal
cargo run -p orbweaver-cli -- graph
cargo run -p orbweaver-cli -- graph --json
```

On this machine that already surfaces a real, non-obvious signal: several
independent repositories (`amber`, `mimic`, `vitruvian`, `volley`) all
declare a dependency on `padagonia`, making it the most depended-on
repository in the scanned estate — the same "graph capability with
multiple downstream consumers" pattern the directive's worked example
(section 5, Padagonia → Bank → Brandi/Amber) describes. Orbweaver did not
need to be told this relationship existed; it read it out of the
manifests.

## What's explicitly not built yet

Everything past Phase I is unimplemented, not stubbed to look otherwise:

- Capability extraction (a function/CLI command/README/test cluster ->
  named capability with confidence and cost/value estimates) — Phase II.
- ELCI connectors (Ontism, Padagonia, Kaptaind, Deckhand, Skillastic,
  Switchboard, Cambrian, Mimic, Goglz, Hellhound, Isopod, Schem) — Phase
  III. `orbweaver doctor` reports this honestly rather than pretending
  connectivity exists.
- Opportunity discovery, the leverage/scoring engine, counterfactual
  comparison, budget-constrained ranking — Phases IV–V.
- LLM reasoning tiers, adversarial critique, evaluation corpus — Phase VI.
- Outcome tracking and calibration — Phase VII.
- Postgres, Axum API, RBAC, policy engine, web UI — Phase VIII.

Building these against fake/imagined ELCI interfaces would violate the
directive's own rule (section 26): connectors must discover their actual
interface at runtime, not assume one. Phase III starts by inspecting what
each real ELCI tool actually exposes.

## Workspace layout

```
crates/
  orbweaver-core      data model (Repository, DependencyRef, errors)
  orbweaver-evidence   Evidence/Confidence/Availability provenance types
  orbweaver-git        deterministic git history inspection (git2)
  orbweaver-ingest     discovery + manifest parsing + dependency resolution
  orbweaver-graph      petgraph projection + JSON export
  orbweaver-storage    SQLite snapshot persistence
  orbweaver-cli        `orbweaver` binary
```

Crates for later phases (`orbweaver-analysis`, `orbweaver-opportunities`,
`orbweaver-scoring`, `orbweaver-reasoning`, `orbweaver-connectors`,
`orbweaver-policy`, `orbweaver-api`, `integrations/*`) are not scaffolded
yet — they'll be added when there's real logic to put in them.
