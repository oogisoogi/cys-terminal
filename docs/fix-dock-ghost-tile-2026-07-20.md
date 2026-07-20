# 과제2: 도크 유령 타일 근본 패치

> 2026-07-20 · cys-dev(node227) · 빌드 미실행(config 파일만 · 전달=소스)

## 원인 (실측)
- `src-tauri/tauri.conf.json`: `productName:"cys"` · `identifier:"com.cysjavis.terminal"`.
- 설치본 `/Applications/cys.app`와 로컬 dev 빌드가 **productName·identifier 동일** →
  macOS LaunchServices가 **같은 앱으로 취급** → 도크에 유령 타일(중복 아이콘).

## 패치
dev 전용 오버레이 config 신설(prod config 무변경):
```
src-tauri/tauri.dev.conf.json = { productName:"cys-dev", identifier:"com.cysjavis.terminal.dev" }
```
dev 빌드 시 `--config`로 병합해 정체성을 분리한다:
```sh
cargo tauri build --config src-tauri/tauri.dev.conf.json
# 또는 dev 실행: cargo tauri dev --config src-tauri/tauri.dev.conf.json
```
→ dev 산출물이 `cys-dev.app` / `com.cysjavis.terminal.dev`로 분리 → LaunchServices가
다른 앱으로 인식 → 유령 타일 소멸.

## 잔재 청소 (빌드 시점에 1회 · 지금은 미실행)
> codex#9 반영: 전역 `lsregister -kill -r`(전체 로컬/유저 DB 재빌드)는 대상 하나보다
> 영향 범위가 과도 → **정확한 stale 번들 경로를 찾아 targeted unregister**한다.
```sh
LSREG=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
# 1) 유령 번들의 실제 경로 확인(정체성 com.cysjavis.terminal 등록 목록):
"$LSREG" -dump | grep -B2 -A6 "com.cysjavis.terminal" | grep "path:"
mdfind "kMDItemCFBundleIdentifier == 'com.cysjavis.terminal'"
# 2) 옛 dev 빌드 산출물만 targeted 제거·등록해제(설치본 /Applications/cys.app 은 건드리지 않음):
rm -rf src-tauri/target/release/bundle src-tauri/target/debug/bundle
"$LSREG" -u "<위에서 확인한 stale dev 번들 경로>"    # -u = 그 경로만 unregister
```

## dev 빌드 wrapper (codex#9 — 정체성 오빌드 차단)
dev 실행/빌드가 **항상** overlay를 쓰고 결과 bundle-id를 검증하도록 래퍼를 둔다
(overlay 누락 시 prod 정체성으로 빌드돼 유령 재발):
```sh
# scripts/dev-build.sh (예시 · 빌드 시점 도입)
set -e
cargo tauri build --config src-tauri/tauri.dev.conf.json "$@"
APP=src-tauri/target/release/bundle/macos/cys-dev.app
ID=$(plutil -extract CFBundleIdentifier raw "$APP/Contents/Info.plist")
[ "$ID" = "com.cysjavis.terminal.dev" ] || { echo "FATAL: bundle id=$ID (overlay 미적용?)"; exit 1; }
```

## ★과제1과의 상호작용 (중요 · codex#5 staged gate)
identifier가 갈리면 **WKWebView localStorage 경로도 분리**된다
(`~/Library/WebKit/com.cysjavis.terminal` ↔ `…terminal.dev`). 즉 dev에서 저장한
레이아웃·폰트 설정이 prod `cys.app`에서 안 보인다. → **과제1의 `~/.cys/ui-layouts.json`
파일 persist가 이 분리를 해소**한다(사용자 설정을 identifier-독립 위치로 이전).

**순서 게이트(codex#5)**: 과제1의 1b(파일 persist)를 **prod identifier에서 먼저 실행해
localStorage→파일 마이그레이션을 확인**한 뒤에 이 identifier 분리를 활성화한다. dev-first로
가면 첫 dev 실행이 빈 화면이 아니라 **기본 layout + "prod에서 가져오는 중" 명시 상태**를
보여야 한다. 두 패치는 이 순서로 함께 가야 UX 회귀가 없다.

## 개발자(yoonsik choi) 전달
소스 커밋으로 포함(바이너리 패치 아님). dev config는 prod 빌드에 영향 0(오버레이는
명시적 `--config` 시에만 병합).
