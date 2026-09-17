# Packaging templates

These are **templates**, not published packages. None of them has been submitted to
`homebrew-core`, the Scoop `extras` bucket, or `winget-pkgs`, and the checksums inside
are placeholders (`0000...`) until a real release is cut and its `checksums.txt` is
available from GitHub Releases.

To cut a real package after a tagged release:

1. Run `scripts/release/package.sh` (or let CI do it) to produce
   `mvs-manager-<version>-<target>.<tar.gz|zip>` archives and `checksums.txt`.
2. Fill in `version` and the per-target `sha256` fields below from that `checksums.txt`.
3. Homebrew: submit `homebrew/mvs-manager.rb` to a tap (`brew tap-new` / a personal tap
   first, then `homebrew-core` once the project meets its notability bar).
4. Scoop: submit `scoop/mvs-manager.json` to the `ScoopInstaller/Extras` bucket, or host
   it in a personal bucket with `scoop bucket add`.
5. winget: run `winget validate` and `winget-create` (or submit by hand) against
   `winget/` to `microsoft/winget-pkgs`.

See [../docs/DISTRIBUTION.md](../docs/DISTRIBUTION.md) for the full channel evaluation.
