# Usage Guide

## 1) Generate or update `mvs.json`

```bash
mvs-manager generate --root . --manifest mvs.json --context cli
```

Machine-readable output:

```bash
mvs-manager generate --root . --manifest mvs.json --context cli --format json
```

Use `--arch-break` for explicit data/schema breaks:

```bash
mvs-manager generate --root . --manifest mvs.json --context cli --arch-break --arch-reason "persistent schema migration"
```

Range strategy flags:

```bash
# lock host/extension protocol ranges to current PROT
mvs-manager generate --root . --manifest mvs.json --context cli --lock-step

# declare compatibility window and auto-generate shim declarations
mvs-manager generate --root . --manifest mvs.json --context cli --backwards-compatible 3
```

Every increment rationale is persisted to `mvs.json.history`.

## 2) Build-gate with linter

```bash
mvs-manager lint --root . --manifest mvs.json
```

Machine-readable output:

```bash
mvs-manager lint --root . --manifest mvs.json --format json
```

Optional AI schema drift checks:

```bash
mvs-manager lint --root . --manifest mvs.json --ai-schema ./tool_schema.json
```

AI liveness checks (runtime capability validation):

```bash
mvs-manager lint --root . --manifest mvs.json --available-model-capabilities tool_calling,json_schema,reasoning-v1
```

## 3) Validate host/extension compatibility

```bash
mvs-manager validate --host-manifest host.json --extension-manifest extension.json --allow-shims true
```

Machine-readable output:

```bash
mvs-manager validate --host-manifest host.json --extension-manifest extension.json --format json
```

Context hierarchies are supported. Example: `edge` extensions can run on `edge.mobile` hosts when ranges and capabilities pass.

AI runtime capability override for validation:

```bash
mvs-manager validate \
  --host-manifest host.json \
  --extension-manifest extension.json \
  --host-model-capabilities tool_calling,reasoning-v1
```

## Makefile shortcuts

```bash
make install-hooks
make generate
make lint-manifest
make validate
make ci
make build-release
```

`make install-hooks` sets `core.hooksPath` to `.githooks` and enables a pre-commit gate that runs `make lint-manifest`.

## Semantic Evidence

`generate` writes both hashes and semantic inventories into `mvs.json.evidence`:

- `feature_inventory`
- `protocol_inventory`
- `public_api_inventory`

`lint` checks these snapshots in addition to the hashes. If your manifest was created before inventories existed, regenerate once to bring it forward.

## Exit Codes

- `0`: success
- `10`: `generate` execution failure
- `20`: `lint` detected drift
- `21`: `lint` execution failure
- `30`: `validate` incompatibility
- `40`: manifest read/parse/write/validation failure
- `70`: output rendering failure

## Troubleshooting

### `Lint failed ... Public API signature drift detected`
- Run `mvs-manager generate` and commit updated `mvs.json`.
- If this is a true integration break, verify `PROT` increment rationale in output.

### `Checksum mismatch`
- Re-download archive + checksums.
- Confirm you are validating the matching release tag.
- If mismatch persists, treat artifact as untrusted.

### `Protocol range mismatch`
- Update host/extension ranges in `mvs.json`.
- Add a `legacy_shims` adapter only if degraded compatibility is intentionally supported.

### Installer cannot find release
- Confirm `MVS_REPO` points to `alextheberge/MVSengine`.
- Ensure tag exists and archive target matches your OS/CPU.
