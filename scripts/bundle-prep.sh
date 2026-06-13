#!/bin/sh
# tauri build 전처리: UI 번들 최신화 + 데몬/CLI 릴리스 빌드 + externalBin 배치.
# tauri.conf.json beforeBuildCommand가 호출한다 (src-tauri 디렉토리 기준 실행).
set -e
cd "$(dirname "$0")/.."

sh ui/build.sh
cargo build --release --bin cys --bin cysd

triple="$(rustc -vV | sed -n 's/^host: //p')"
mkdir -p src-tauri/binaries
cp "target/release/cys" "src-tauri/binaries/cys-$triple"
cp "target/release/cysd" "src-tauri/binaries/cysd-$triple"
echo "bundle-prep ready (ui/dist + binaries for $triple)"
