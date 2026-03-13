# Release Local Artifact Bootstrap Implementation

Goal: make local release-doc governance executable end-to-end by regenerating derived `.docs/`
artifacts from the tracked release documents.

Status: Implemented and validated on the current task branch

## Delivered

1. Added `scripts/bootstrap_release_local_artifacts.sh`.
2. Added `scripts/test_bootstrap_release_local_artifacts.sh`.
3. Updated `docs/releases/README.md` to point contributors at the bootstrap flow.

## Minimal Production Change

Modified:

- `scripts/bootstrap_release_local_artifacts.sh`
- `scripts/test_bootstrap_release_local_artifacts.sh`
- `docs/releases/README.md`
- `docs/plans/2026-03-13-release-local-artifact-bootstrap-design.md`
- `docs/plans/2026-03-13-release-local-artifact-bootstrap.md`

Behavior:

1. scan released versions from `CHANGELOG.md`
2. read `Trace ID` and `Trace path` from each tracked release doc
3. regenerate:
   - `.docs/releases/vX.Y.Z-debug.md`
   - `.docs/traces/index.jsonl`
   - `.docs/traces/latest`
   - `.docs/traces/by-tag/vX.Y.Z/latest`
   - `${trace_path}/metadata.json`
4. fail fast if the release doc is missing required trace linkage or if the trace path is outside
   `.docs/traces/`
5. fail strict doc governance when a release doc keeps inconsistent trace/debug linkage between its
   summary fields and `## Detail Links`
6. fail strict doc governance and bootstrap when the summary `Trace ID` does not match the trace
   directory basename
7. fail strict doc governance when existing local `.docs` artifacts drift away from the tracked
   release doc even if the files still exist

## Validation

Commands completed after the change:

```bash
bash scripts/test_bootstrap_release_local_artifacts.sh
scripts/bootstrap_release_local_artifacts.sh
LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh
git diff --check
```

Observed results:

- bootstrap regression: PASS
- local artifact bootstrap on the active repo: PASS
- strict local doc governance: PASS
- `git diff --check`: PASS

## Outcome

Local release governance is no longer a policy that only CI can satisfy cleanly.

The repository now has a deterministic, test-backed path to rehydrate the ignored `.docs/`
artifacts from the tracked release record without broadening CI or weakening the checks.
