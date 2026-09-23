# Contributing to RunwayBar

Thanks for helping! A few project-specific rules:

## Commits

- [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `chore:`, …).
- **No co-author or attribution trailers.** Do not add `Co-Authored-By: …` or
  `Generated with …` lines; CI rejects them.

## Hard invariants (CI-enforced where possible)

- **Read-only credentials.** Never write, rename, or refresh the provider tools'
  credential files. `scripts/strace-readonly-check.sh` guards this.
- **No secrets in output.** Tokens go through `SecretString`; never into logs,
  errors, JSON, or argv.
- **Unknown stays unknown.** Missing data renders as unknown, never 0% or 100%.
- **Vendor marks.** Do not use vendor names in product naming or icons; provider
  names appear only as factual descriptors.

## Development

```sh
cargo test                 # full suite
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
scripts/strace-readonly-check.sh
```

Landing a release: tag `vX.Y.Z`; CI builds artifacts, checksums and attestations,
and opens a draft release for review.
