#!/bin/sh
# tauri build 전처리: UI 번들 최신화 + 데몬/CLI 릴리스 빌드 + externalBin 배치.
# tauri.conf.json beforeBuildCommand가 호출한다 (src-tauri 디렉토리 기준 실행).
# CYS_TARGET(예: x86_64-apple-darwin) 설정 시 그 타깃으로 크로스 빌드 — CI 매트릭스용.
# 미설정 시 호스트 타깃으로 빌드 — 로컬 빌드 동작 그대로 유지.
set -e
cd "$(dirname "$0")/.."

sh ui/build.sh

if [ -n "$CYS_TARGET" ]; then
  triple="$CYS_TARGET"
  cargo build --release --target "$triple" --bin cys --bin cysd
  bindir="target/$triple/release"
else
  triple="$(rustc -vV | sed -n 's/^host: //p')"
  cargo build --release --bin cys --bin cysd
  bindir="target/release"
fi

mkdir -p src-tauri/binaries
cp "$bindir/cys" "src-tauri/binaries/cys-$triple"
cp "$bindir/cysd" "src-tauri/binaries/cysd-$triple"
echo "bundle-prep ready (ui/dist + binaries for $triple)"
