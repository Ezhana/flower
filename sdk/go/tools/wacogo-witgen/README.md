# Flower WIT binding generator

This directory is a source-pinned fork of `github.com/partite-ai/wacogo/internal/witgen` at commit `1fd5d8bf78a7`, retained under its Apache-2.0 license.

Flower carries the fork because that revision emits invalid bindings for two valid Canonical ABI shapes used by `workflow-engine`:

- nominal declarations are grouped by kind instead of topologically ordered, so records that reference variants use undeclared Go variables;
- wrapper calls always flatten parameters, even when the Canonical ABI requires the parameter tuple to be passed indirectly after 16 flat slots.

The local generator topologically orders nominal declarations and lowers oversized parameter tuples into callee memory. `sdk/go/internal/componentabi/generate.go` invokes it directly. Generated bindings are a single deterministic output; no post-generation mutation is permitted. `scripts/verify.sh` snapshots the checked-in bindings, regenerates them, and fails on any drift.
