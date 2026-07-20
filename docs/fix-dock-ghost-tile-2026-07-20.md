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
같은 정체성으로 이미 빌드된 옛 번들이 LaunchServices DB에 등록돼 있으면 유령이 남는다:
```sh
rm -rf src-tauri/target/release/bundle src-tauri/target/debug/bundle
# LaunchServices 재빌드(등록 잔재 제거):
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -kill -r -domain local -domain user
```

## ★과제1과의 상호작용 (중요)
identifier가 갈리면 **WKWebView localStorage 경로도 분리**된다
(`~/Library/WebKit/com.cysjavis.terminal` ↔ `…terminal.dev`). 즉 dev에서 저장한
레이아웃·폰트 설정이 prod `cys.app`에서 안 보인다. → **과제1의 `~/.cys/ui-layouts.json`
파일 persist가 이 분리를 해소**한다(사용자 설정을 identifier-독립 위치로 이전).
두 패치는 함께 가야 UX 회귀가 없다.

## 개발자(yoonsik choi) 전달
소스 커밋으로 포함(바이너리 패치 아님). dev config는 prod 빌드에 영향 0(오버레이는
명시적 `--config` 시에만 병합).
