#!/usr/bin/env bash
# Local release: signed app + DMG, notarized and stapled.
#
#   make release                       # uses the defaults below
#   NOTARY_PROFILE=xxx make release    # different notarytool keychain profile
#
# Credentials, either of:
#   a) a .env file (repo root or desktop/, git-ignored) with
#        APPLE_SIGNING_IDENTITY="Developer ID Application: ..."   (optional)
#        APPLE_ID=<apple id>  APPLE_PASSWORD=<app-specific pw>  APPLE_TEAM_ID=HQ537XMLJY
#   b) a notarytool keychain profile (one-time):
#        xcrun notarytool store-credentials tidy-notary \
#          --apple-id <apple id> --team-id HQ537XMLJY --password <app-specific pw>
#
# Steps: tauri build (codesigns Tidy.app with the Developer ID + hardened
# runtime) → stamp the DMG icon → codesign the DMG → notarize → staple →
# gatekeeper self-check. Everything after the build is skipped with a clear
# message when the notary profile is missing, so the signed DMG is still
# produced.
set -euo pipefail

cd "$(dirname "$0")/.."   # desktop/

# Load credentials from .env (desktop/.env wins over the repo root one).
for envfile in ../.env .env; do
  if [ -f "$envfile" ]; then
    echo "▶ loading $envfile"
    set -a; . "$envfile"; set +a
  fi
done

IDENTITY="${APPLE_SIGNING_IDENTITY:-Developer ID Application: Jiangwei Lan (HQ537XMLJY)}"
PROFILE="${NOTARY_PROFILE:-tidy-notary}"

# 自更新包的 minisign 私钥。没有它 tauri 不会产出 .app.tar.gz.sig，
# publish 这一步就没东西可发（release.sh 本身仍然能出签名 DMG）。
export TAURI_SIGNING_PRIVATE_KEY_PATH="${TAURI_SIGNING_PRIVATE_KEY_PATH:-$HOME/.tauri/tidy-updater.key}"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
if [ ! -f "$TAURI_SIGNING_PRIVATE_KEY_PATH" ]; then
  echo "⚠ 找不到更新签名私钥 $TAURI_SIGNING_PRIVATE_KEY_PATH —— 这次不会产出自更新包。" >&2
  unset TAURI_SIGNING_PRIVATE_KEY_PATH
fi

echo "▶ building + signing with: $IDENTITY"
export APPLE_SIGNING_IDENTITY="$IDENTITY"
npm ci --prefix ui
ui/node_modules/.bin/tauri build

APP="$(ls -d target/release/bundle/macos/*.app | head -1)"
DMG="$(ls target/release/bundle/dmg/*.dmg | head -1)"
echo "▶ app: $APP"
echo "▶ dmg: $DMG"

echo "▶ verifying app signature"
codesign --verify --deep --strict --verbose=2 "$APP"

# The icon lands in the DMG's resource fork; sign the DMG AFTER that so the
# signature covers the final file.
scripts/set-dmg-icon.sh "$DMG"
echo "▶ signing dmg"
codesign --force --sign "$IDENTITY" --timestamp "$DMG"

# Pick the notary auth: explicit Apple ID vars (from .env) or the keychain
# profile. The password never appears in argv logs beyond this process.
if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  NOTARY_AUTH=(--apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID")
elif xcrun notarytool history --keychain-profile "$PROFILE" >/dev/null 2>&1; then
  NOTARY_AUTH=(--keychain-profile "$PROFILE")
else
  cat >&2 <<MSG

⚠ no notary credentials — DMG is signed but NOT notarized.
  Either put APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID in desktop/.env, or store a profile:
    xcrun notarytool store-credentials $PROFILE \\
      --apple-id <your-apple-id> --team-id HQ537XMLJY --password <app-specific-password>
  then re-run: make release
MSG
  exit 2
fi

echo "▶ notarizing (this waits for Apple, usually 1–5 min)"
xcrun notarytool submit "$DMG" "${NOTARY_AUTH[@]}" --wait

echo "▶ stapling"
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"

echo "▶ gatekeeper check"
spctl --assess --type open --context context:primary-signature -v "$DMG"
echo "✅ release ready: $DMG"
echo "   下一步：make publish  # 生成 latest.json 并发到 GitHub Releases"
