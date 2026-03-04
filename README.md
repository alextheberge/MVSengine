# MVS Engine (`mvs-manager`)

Cross-platform CLI implementing multidimensional versioning: `[ARCH].[FEAT].[PROT]-[CONT]`.

## Why Post-SemVer for Apps
- Separate data breaks from integration breaks: `ARCH` and `PROT` move independently.
- Stop silent plugin/SDK regressions: `lint` fails when public API or AI contract drift is unaccounted.
- Keep compatibility explicit: protocol ranges, legacy shims, capabilities, and context live in `mvs.json`.

## Quick Start (4 Steps)

### 1) Install `mvs-manager`

### macOS/Linux
```bash
curl -fsSL https://raw.githubusercontent.com/alextheberge/MVSengine/main/scripts/install.sh | bash
```

### Windows PowerShell
```powershell
irm https://raw.githubusercontent.com/alextheberge/MVSengine/main/scripts/install.ps1 | iex
```

### 2) Initialize your project manifest
```bash
mvs-manager generate --root . --manifest mvs.json --context cli
```

### 3) Add decorators in your code
Decorator syntax can be function-style or `:` style:

```ts
// @mvs-feature("offline_storage")
// @mvs-protocol("auth-api-v1")
export function login(user: string): string {
  return user;
}
```

```rs
/// @mvs-feature("runtime_bridge")
/// @mvs-protocol("host_extension_handshake")
pub fn handshake(version: u32) -> bool {
    version > 0
}
```

### 4) Run the initialize/verify/reconcile loop
```bash
# verify current code matches mvs.json
mvs-manager lint --root . --manifest mvs.json

# after code/decorator/API changes, reconcile and persist rationale/history
mvs-manager generate --root . --manifest mvs.json --context cli
```

## Enforce MVS on Commit
```bash
make install-hooks
```

This installs a repo-managed `pre-commit` hook that runs `make lint-manifest`.

## Local Build
```bash
make ci
make build-release
```

## Core Commands
```bash
mvs-manager generate --root . --manifest mvs.json --context cli
mvs-manager generate --root . --manifest mvs.json --context edge.mobile --backwards-compatible 3
mvs-manager lint --root . --manifest mvs.json
mvs-manager lint --root . --manifest mvs.json --available-model-capabilities tool_calling,json_schema,reasoning-v1
mvs-manager validate --host-manifest host.json --extension-manifest extension.json
mvs-manager validate --host-manifest host.json --extension-manifest extension.json --host-model-capabilities tool_calling,reasoning-v1
```

`mvs.json` persists version-change rationale in `history`, enabling compatibility reports to explain protocol breaks (for example, auth-flow changes tied to a specific `PROT`).

## Release + Verification

Release assets include:
- per-platform archives
- `checksums.txt`
- optional `checksums.txt.asc` GPG signature

Verify checksum:
```bash
scripts/verify-release.sh dist/vX.Y.Z/mvs-manager-X.Y.Z-<target>.tar.gz dist/vX.Y.Z/checksums.txt
```

See [docs/USAGE.md](docs/USAGE.md) and [docs/RELEASE.md](docs/RELEASE.md) for complete workflows.

## License

This repository is licensed under the GNU Affero General Public License v3.0 only (`AGPL-3.0-only`).

- Full license text: [LICENSE](LICENSE)
- If you modify and run this software for users over a network, AGPL requires offering corresponding source to those users.
