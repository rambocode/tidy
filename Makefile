# Tidy standalone Makefile — adapted from the original Mole repo's desktop-*
# targets so the README's `make desktop-*` commands also work in this checkout.
.PHONY: dev build bundle release publish check desktop-dev desktop-build desktop-bundle desktop-check

# The tauri CLI must run from desktop/ (it locates src-tauri/tauri.conf.json
# by scanning subfolders of the invocation directory).
dev:
	cd desktop && npm install --prefix ui && ui/node_modules/.bin/tauri dev

build:
	cd desktop && npm ci --prefix ui && ui/node_modules/.bin/tauri build --no-bundle

# Full bundle: Tidy.app + DMG, both carrying icons/icon.icns. The extra
# set-dmg-icon step themes the .dmg FILE; tauri only themes the mounted volume.
bundle:
	cd desktop && npm ci --prefix ui && ui/node_modules/.bin/tauri build
	desktop/scripts/set-dmg-icon.sh desktop/target/release/bundle/dmg/*.dmg

# Signed + notarized DMG (see desktop/scripts/release.sh for the one-time
# notarytool credential setup).
release:
	desktop/scripts/release.sh

# 把 make release 的产物发到 GitHub Releases，并生成自更新 feed。
publish:
	desktop/scripts/publish.sh

check:
	cd desktop && cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace
	cd desktop/ui && npm run check

# Parent-repo-compatible aliases.
desktop-dev: dev
desktop-build: build
desktop-bundle: bundle
desktop-check: check
