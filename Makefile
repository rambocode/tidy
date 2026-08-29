# Tidy standalone Makefile — adapted from the original Mole repo's desktop-*
# targets so the README's `make desktop-*` commands also work in this checkout.
.PHONY: dev build check desktop-dev desktop-build desktop-check

# The tauri CLI must run from desktop/ (it locates src-tauri/tauri.conf.json
# by scanning subfolders of the invocation directory).
dev:
	cd desktop && npm install --prefix ui && ui/node_modules/.bin/tauri dev

build:
	cd desktop && npm ci --prefix ui && ui/node_modules/.bin/tauri build --no-bundle

check:
	cd desktop && cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace
	cd desktop/ui && npm run check

# Parent-repo-compatible aliases.
desktop-dev: dev
desktop-build: build
desktop-check: check
