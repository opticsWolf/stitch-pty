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

```bash
cargo publish            # Rust crate, from the green version commit
uvx maturin build --release
# upload dist/* to PyPI, tag vX.Y.Z, create the GitHub release
```

## Downstream

- Bump Kilim's `stitch-pty` dependency after publish.
- Never merge `dev` → `main`, tag, or publish without explicit confirmation.
