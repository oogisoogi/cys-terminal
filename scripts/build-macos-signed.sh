#!/usr/bin/env bash
# build-macos-signed.sh — Apple Developer ID 서명 + 공증(notarization) + staple 자동 빌드.
# (오너 2026-06-15) ad-hoc 빌드는 다른 맥으로 전송하면 quarantine + 미공증으로 macOS가
# "손상됨(damaged)"으로 차단한다. Developer ID 인증서 + notarytool 자격증명이 있으면 Tauri가
# 빌드 중 자동으로 codesign(hardened runtime) + notarytool 공증 + stapler staple 한다.
# 이 스크립트는 자격증명을 fail-closed로 검증하고, 빌드 후 Gatekeeper 통과를 실측 확인한다.
#
# 사전(1회 셋업):
#   1) Apple Developer Program 가입($99/년) → "Developer ID Application" 인증서 발급·Keychain 설치
#   2) appleid.apple.com 에서 app-specific password 발급 (또는 App Store Connect API key)
#   3) 아래 env 설정
# 사용:
#   export APPLE_SIGNING_IDENTITY="Developer ID Application: NAME (TEAMID)"
#   export APPLE_ID="you@example.com" APPLE_PASSWORD="xxxx-xxxx-xxxx-xxxx" APPLE_TEAM_ID="TEAMID"
#   #   (또는 API key: APPLE_API_KEY_PATH=AuthKey_XXXX.p8 · APPLE_API_KEY=KEYID · APPLE_API_ISSUER=ISSUER)
#   export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/cys-updater.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
#   scripts/build-macos-signed.sh
# exit 0=서명·공증·검증 통과 / 1=공증 검증 실패 / 2=자격증명·환경 미비
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
VERSION=$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed -E 's/.*"([0-9][0-9.]*)".*/\1/')

# ── 자격증명 fail-closed 검증 ──
: "${APPLE_SIGNING_IDENTITY:?필요: export APPLE_SIGNING_IDENTITY='Developer ID Application: NAME (TEAMID)'}"
if [ -n "${APPLE_API_KEY:-}" ]; then
  : "${APPLE_API_ISSUER:?APPLE_API_KEY 사용 시 APPLE_API_ISSUER 필요}"
  echo "공증 자격: App Store Connect API key"
else
  : "${APPLE_ID:?공증용 Apple ID 필요 (또는 APPLE_API_KEY 경로)}"
  : "${APPLE_PASSWORD:?app-specific password 필요 (APPLE_PASSWORD)}"
  : "${APPLE_TEAM_ID:?APPLE_TEAM_ID 필요}"
  echo "공증 자격: Apple ID($APPLE_ID) + app-specific password"
fi
command -v xcrun >/dev/null || { echo "✗ Xcode Command Line Tools 필요(xcrun) — xcode-select --install"; exit 2; }
if ! security find-identity -v -p codesigning 2>/dev/null | grep -q "Developer ID Application"; then
  echo "✗ Keychain에 'Developer ID Application' 인증서 없음 — Apple Developer에서 발급·설치 필요"; exit 2
fi
[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ] || echo "⚠ TAURI_SIGNING_PRIVATE_KEY 미설정 — 자동업데이트 .sig 미생성(설치 DMG는 정상)"

echo "== Apple 공증 빌드 v$VERSION (Tauri 자동 codesign[hardened]+notarize+staple) =="
bun x @tauri-apps/cli build

APP="target/release/bundle/macos/cys.app"
DMG="target/release/bundle/dmg/cys_${VERSION}_aarch64.dmg"

echo "== 검증: Gatekeeper(spctl) + 공증 티켓(stapler) =="
if spctl -a -vv "$APP" 2>&1 | grep -qi "accepted"; then
  echo "  ✓ spctl: accepted (다른 맥에서도 경고 없이 열림 — '손상됨' 해소)"
else
  echo "  ✗ spctl 거부 — 공증 실패. 위 빌드 로그의 notarization 결과를 확인하라"; exit 1
fi
xcrun stapler validate "$APP" >/dev/null 2>&1 && echo "  ✓ app 공증 티켓 stapled" || echo "  ⚠ app staple 미확인"
xcrun stapler validate "$DMG" >/dev/null 2>&1 && echo "  ✓ DMG 공증 티켓 stapled" || echo "  ⚠ DMG staple 미확인(앱 공증되면 설치는 정상)"

echo "== 배포본 정리 + 자동업데이트 매니페스트 =="
cp "$DMG" "dist-mac/cys-${VERSION}-macos-arm64.dmg"
sh scripts/make-update-manifest.sh "$VERSION" idoforgod cys-terminal >/dev/null
echo "✓ 공증 빌드 완료: dist-mac/cys-${VERSION}-macos-arm64.dmg"
echo "  → ad-hoc 재서명·xattr 우회 불필요. gh release 발행은 오너 승인 후."
