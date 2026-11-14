# Releasing stitch-pty

Rule: **a version is released from `dev` only when all suites are green on
the exact commit that carries the bump.** No "bump now, fix later" commits.

## Version-number rules (from `docs/IMPROVEMENT_PLAN.md`)

| Change class | Bump |
|---|---|
| Bug fix, test-only, CI/chore, docs | `+0.0.1` (patch) |
| New feature, major refactor | `+0.1.0` (minor) |

After a minor bump, the patch sequence continues under the new minor
(`0.6.0 → 0.6.1 → …`). One version = one commit = one push to `dev`.

## Per-version checklist (runnable)

```bash
cd /d/User/Documents/Python/stitch-pty   # (Git Bash; adjust for your shell)
git checkout dev && git status --short    # must be clean before starting
```

1. **Bump all three version fields** to `X.Y.Z` (they must agree at every commit):
   - `Cargo.toml` → `version = "X.Y.Z"`
   - `pyproject.toml` → `version = "X.Y.Z"`
   - `python/stitch_pty/__init__.py` → `__version__ = "X.Y.Z"`
2. **Rust suite:** `cargo test --lib` → 0 failures.
3. **Cross-platform lints:** `cargo clippy --all-targets --target
   x86_64-unknown-linux-gnu -- -D warnings` and `--target
   aarch64-apple-darwin` → 0 errors. (The Windows toolchain never compiles
   `platform_unix.rs`, so Unix-only lints hide from it — v0.7.6 repaired
   exactly this CI failure. `rustup target add <triple>` once to install.)
3. **Rebuild the extension:** `uvx maturin develop --release`
   (requires a venv: `uv venv` if none exists).
4. **Python suite:** `uv run python -m pytest tests/ -q`
   (parallel: `uv run --with pytest-xdist python -m pytest tests/ -q -n auto`).
5. **Changelog:** append a row under the new version heading in `CHANGELOG.md`.
6. **Commit:** `<type>: <summary> (vX.Y.Z)` — one commit, nothing else mixed in.
7. **Push:** `git push origin dev`.

If any suite is red, fix forward **within the same version** before pushing.

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

Then create the GitHub release from the matching CHANGELOG section:

```bash
# Extract the vX.Y.Z section to a notes file, then:
gh release create vX.Y.Z --title "vX.Y.Z — <summary>" --notes-file /tmp/notes.md
```

If a publish job fails (for example, a stale `CARGO_REGISTRY_TOKEN`), fix the
secret or workflow and rerun that job; do not move the tag. If the tagged
commit itself is red, fix forward in a new patch version.

A release tag may point at a docs-only commit after the green version-bump
commit (the 0.10.x convention), as long as the version fields, CHANGELOG, and
tests are identical to the green commit.

## Downstream

- Bump Kilim's `stitch-pty` dependency after publish.
- Never merge `dev` → `main`, tag, or publish without explicit confirmation.
