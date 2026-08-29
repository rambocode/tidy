# Repository Guidelines

## Project Structure & Module Organization

Tidy contains a macOS Tauri application and its static product site. `desktop/crates/` holds the Rust safety and operation layers, `desktop/src-tauri/` exposes thin IPC commands, and `desktop/ui/src/` contains the framework-free TypeScript UI. `site/` is the deployable website: edit articles in `site/content/blog/`, shared assets in `site/assets/`, and publishing tools in `site/tools/`. The root `tests/fuzz_corpus/` supplies adversarial paths to desktop safety tests. Before changing desktop code, read `desktop/AGENTS.md`; it is authoritative for deletion safety and desktop-specific review requirements.

## Build, Test, and Development Commands

Run these from the repository root:

```bash
make dev       # install UI dependencies and launch Tauri
make check     # Rust format, Clippy, tests, and TypeScript checks
make build     # compile an unsigned app without bundling
```

For website content, run `cd site/tools && npm install`, then `npm run check` to validate frontmatter, links, media, and routes, or `npm run build` to regenerate blog pages, RSS, and the sitemap. Rust stable and Node 20+ are required.

## Coding Style & Naming Conventions

Rust uses edition 2021, four-space indentation, `snake_case` modules, `cargo fmt`, and warning-free Clippy. TypeScript is strict; follow nearby formatting and use lowercase, responsibility-based filenames such as `views/clean.ts`. Keep Tauri commands thin and preserve the existing core-to-operations-to-IPC/UI dependency direction. Treat `site/content/blog/` as the article source of truth; never hand-edit generated `site/{zh,en}/blog/` pages.

## Testing Guidelines

Keep focused Rust unit tests beside the protected module in `#[cfg(test)]` blocks. Cover refusal paths and preview-to-execution invariants when safety behavior changes. There is no numerical coverage target. During iteration, use `cd desktop && cargo test -p mole-core <filter>`; before review, run `make check` and the site validator when site content changed.

## Commits & Pull Requests

This checkout has no Git metadata, so follow the documented Conventional Commit convention: `fix(clean): reject unsafe candidates` or `feat(site): add safety article`. Keep commits scoped. PRs must summarize user-visible behavior, safety implications, and validation run; link relevant issues, include screenshots for UI changes, and call out IPC, permission, log-format, or deletion-policy changes explicitly.
