# MVS Engine (`mvs-manager`)

Cross-platform CLI implementing multidimensional versioning: `[ARCH].[FEAT].[PROT]-[CONT]`.

## Install

### macOS/Linux
```bash
curl -fsSL https://raw.githubusercontent.com/<owner>/<repo>/main/scripts/install.sh | MVS_REPO=<owner>/<repo> bash
```

### Windows PowerShell
```powershell
irm https://raw.githubusercontent.com/<owner>/<repo>/main/scripts/install.ps1 | iex
# set MVS_REPO first
```

## Local Build
```bash
make ci
make build-release
```

## Core Commands
```bash
mvs-manager generate --root . --manifest mvs.json --context cli
mvs-manager lint --root . --manifest mvs.json
mvs-manager validate --host-manifest host.json --extension-manifest extension.json
```

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
