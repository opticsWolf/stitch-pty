# Releasing stitch-pty

Rule: **a release is published only from a commit that contains the version
bump and has all suites green.** No "bump now, fix later" release. Follow-up
fixes/tests/docs may be separate commits before the tag, but never rewrite a
pushed bump or push a red one.

## Version-number rules (from `docs/IMPROVEMENT_PLAN.md`)

| Change class | Bump |
|---|---|
| Bug fix, test-only, CI/chore, docs | `+0.0.1` (patch) |
| New feature, major refactor | `+0.1.0` (minor) |

After a minor bump, the patch sequence continues under the new minor
(`0.6.0 → 0.6.1 → …`). Each version has one version commit (implementation +
bump/changelog); follow-up fixes may land before release, and the tag must
point at a green commit that contains the bump.

## Per-version checklist (runnable)

```bash
cd /d/User/Documents/Python/stitch-pty   # (Git Bash; adjust for your shell)
git checkout dev && git status --short    # must be clean before starting
```

1. **Bump all three version fields** to `X.Y.Z` (they must agree at every commit):
   - `Cargo.toml` → `version = "X.Y.Z"`
   - `pyproject.toml` → `version = "X.Y.Z"`
   - `python/stitch_pty/__init__.py` → `__version__ = "X.Y.Z"`
   Refresh `Cargo.lock`/`uv.lock` and the README install/feature snippets.
2. **Format:** `cargo fmt --all --check`.
3. **Cross-platform lints:** `cargo clippy --all-targets -- -D warnings`,
   `cargo clippy --all-targets --target x86_64-unknown-linux-gnu -- -D warnings`,
   and `cargo clippy --all-targets --target aarch64-apple-darwin -- -D warnings`
   (plus `--features regex` when that feature changed). The Windows toolchain
   never compiles `platform_unix.rs`, so Unix-only lints hide from it —
   v0.7.6 repaired exactly this CI failure. `rustup target add <triple>` once.
4. **Rust suite:** `cargo test` → 0 failures (unit + integration + doctest).
5. **No-PyO3 core:** `cargo check --no-default-features --all-targets`,
   `cargo test --no-default-features --lib`, and
   `cargo test --no-default-features --features regex --lib` when `expect` or
   regex changed. CI also asserts `pyo3` is absent from the default tree.
6. **Python-facing checks:** `cargo check --features python`,
   `cargo doc --no-deps`, rebuild with `maturin develop --release`, then:
   - `python -m pytest tests/ -q`
   - `ruff check python/ tests/`
   - `mypy python/stitch_pty/`
7. **Changelog:** add the version section in `CHANGELOG.md`; keep
   `[Unreleased]` and update its comparison link.
8. **Commit:** `<type>: <summary> (vX.Y.Z)` for the version commit; follow-up
   fixes/docs can be separate commits.
9. **Push:** `git push origin dev`; wait for CI on the exact commit.

If any suite is red, fix forward **within the same version** before tagging.

## Publishing (crates.io + PyPI) — requires explicit maintainer go-ahead

The release is tag-driven. Once the maintainer authorizes the release:

```bash
git checkout dev
git pull --ff-only
git checkout main
git merge --ff-only dev          # main must be a fast-forward
git tag vX.Y.Z                    # lightweight tag, matching v0.10.x
git push origin main
git push origin vX.Y.Z
```

The `v*` tag triggers `.github/workflows/CI.yml`, which:

1. runs the full test/lint matrix on the tagged commit;
2. builds the Linux x86_64/aarch64, macOS universal2, and Windows wheels;
3. publishes to PyPI via trusted publishing;
4. publishes the Rust crate to crates.io.

Then create the GitHub release from the matching CHANGELOG section. Use a
Windows-resolvable notes path (`C:/Users/.../Temp/notes.md`) if running from
Git Bash on Windows; `/tmp/...` is not the same filesystem for `gh`:

```bash
gh release create vX.Y.Z --title "vX.Y.Z — <summary>" --notes-file <notes.md>
```

If a publish job fails (for example, a stale `CARGO_REGISTRY_TOKEN`), fix the
secret or workflow and rerun that job; do not move the tag. If the tagged
commit itself is red, fix forward in a new patch version.

A release tag may point at a docs-only commit after the green version commit
(the 0.10.x convention), as long as the version fields, CHANGELOG, and tests
are identical to the green commit.

### Releasing several implemented minors together

If several minor versions were implemented but not published (for example
0.11.0–0.15.0), release **one** final version (for example `v0.15.0`) unless
there is a reason to publish each. In that case:

- mark the intermediate CHANGELOG headings as unpublished increments folded
  into the final release;
- keep all implementation notes and test counts in the final release notes;
- do not create retroactive tags for the skipped versions;
- do not imply that PyPI/crates.io ever had the intermediate versions.

## Downstream

- A Kilim `stitch-pty` dependency bump is optional and currently deferred; do
  it only when explicitly requested.
- Never merge `dev` → `main`, tag, or publish without explicit confirmation.
