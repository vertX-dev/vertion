# Packaging templates

Nothing in this directory is used by a build. These are the manifests other
package managers need, kept here so they stay next to the release workflow whose
asset names they depend on.

**None of them work until a release exists.** They all point at GitHub release
assets, and every one carries a `REPLACE_...` placeholder where a checksum goes.

| File | For | Lives where, in the end |
| --- | --- | --- |
| `homebrew/vertion.rb` | `brew install` | A separate `homebrew-tap` repo, as `Formula/vertion.rb` |
| `scoop/vertion.json` | `scoop install` | A separate `scoop-bucket` repo, as `bucket/vertion.json` |

`cargo binstall` needs no file here — its metadata lives in `Cargo.toml` under
`[package.metadata.binstall]`, and it works as soon as a release is published.

## The coupling to watch

All three depend on the exact asset names that
`.github/workflows/release.yml` produces:

```
vertion-v<version>-<target>.tar.gz     # unix
vertion-v<version>-<target>.zip        # windows
```

and on each archive containing a single top-level directory of the same name,
with the binary inside it. **If you rename the release assets, all three break
at once** — the `bin-dir` in `Cargo.toml`, the Homebrew `url`s, and Scoop's
`extract_dir`.

## Cutting a release

1. Tag `vX.Y.Z` and let the workflow build the draft release.
2. Publish the draft.
3. `cargo binstall` works immediately — nothing to update.
4. For Homebrew and Scoop, take the checksums from the `.sha256` assets, update
   `version` and the hashes, and push to the tap or bucket repo.

Scoop's `autoupdate` block reads `$url.sha256` on its own, so after the first
manual publish its bucket keeps itself current. Homebrew has no equivalent;
`brew bump-formula-pr` is the usual way to update a formula.
