# Usage Guide

## 1) Generate or update `mvs.json`

```bash
mvs-manager generate --root . --manifest mvs.json --context cli
```

Use `--arch-break` for explicit data/schema breaks:

```bash
mvs-manager generate --root . --manifest mvs.json --context cli --arch-break --arch-reason "persistent schema migration"
```

## 2) Build-gate with linter

```bash
mvs-manager lint --root . --manifest mvs.json
```

Optional AI schema drift checks:

```bash
mvs-manager lint --root . --manifest mvs.json --ai-schema ./tool_schema.json
```

## 3) Validate host/extension compatibility

```bash
mvs-manager validate --host-manifest host.json --extension-manifest extension.json --allow-shims true
```

## Makefile shortcuts

```bash
make generate
make lint-manifest
make validate
make ci
make build-release
```

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
