# Contributing

## Setup

Use the repo dev shell:

```bash
nix develop
just
```

## Common Commands

```bash
just test
just test-render
just eval
just lint
```

## Change Guidance

When changing API behavior:

- update `docs/api.md`
- keep `schema_version` on machine-readable payloads
- preserve legacy compatibility unless there is a clear migration plan

When changing renderer behavior:

- add or update render unit tests in `server/src/render.rs`
- update eval fixtures or goldens with `just eval-update` when the change is intentional

When changing docs/examples:

- keep `README.md`, `AGENTS.md`, and `docs/api.md` aligned with the actual implementation

## Validation

Recommended order:

```bash
just test-render
just eval
just test
just lint
```

## Pull Request Checklist

- code builds in `nix develop`
- relevant tests added or updated
- `just eval` passes
- docs updated for API or workflow changes
