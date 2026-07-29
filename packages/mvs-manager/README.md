# `mvs-manager` npm package

Thin Node wrapper that downloads the matching GitHub Release binary for your OS (same archives and `checksums.txt` SHA-256 verification as [`scripts/install.sh`](../../scripts/install.sh)).

## Install

```bash
npm install --save-dev mvs-manager@2.0.0
```

Pin with `MVS_VERSION` / package version. Forks set `MVS_REPO=owner/name`.

Skip download (CI cache / offline):

```bash
MVS_SKIP_BINARY_DOWNLOAD=1 npm install
```

## Usage

```bash
npx mvs-manager lint --root . --manifest mvs.json
npx mvs-manager generate --root . --manifest mvs.json --context cli --fix
```

Bug-fix releases: `--fix` bumps the FIX axis without FEAT/PROT drift. Package SemVer is `ARCH.FEAT.FIX`.

## License

AGPL-3.0-only (same as MVS Engine).
