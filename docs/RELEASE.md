# 릴리스 절차 (cys 터미널)

> **현행 표준 절차(2026-07 정정)**: 릴리스는 **release.yml 자동화**가 정본이다 —
> ①버전 범프(아래 §0 4곳)+`cargo build`(Cargo.lock)+로컬 `bash scripts/secret-scan.sh --all` clean 확인
> ②main push ③`git tag vX.Y.Z && git push origin vX.Y.Z`(태그=오너 직접·가드)
> ④CI 4잡(mac signed·mac x86 sidecar·**windows NSIS**·pack) green + windows-build.yml T5 green
> ⑤릴리스 자산·`latest.json`(tauri v2 — darwin-aarch64·darwin-x86_64·windows-x86_64 3키) 실측 확인.
> Windows 인스톨러는 **NSIS**다(`src-tauri/tauri.windows.conf.json targets:["nsis"]`) — 아래 §2·부록의
> 수동 MSI/WiX 경로는 **legacy(폐기·참고용)**이며 따르지 마라.

## 0. 버전 위치 (범프 시 모두 갱신 — 실측 4곳)

- `Cargo.toml` / `src-tauri/Cargo.toml` — `version`
- `src-tauri/tauri.conf.json` — `version`
- `ui/package.json` — `version`
- ~~`dist-win/cys.wxs` / `dist-win/cys-x64.wxs`~~ (legacy MSI — NSIS 전환으로 폐기)

## 1. macOS 빌드 (DMG + 앱 번들 + 업데이트 아티팩트)

> **자동 업데이트가 켜져 있으므로(`createUpdaterArtifacts: true`) 빌드 시 서명 키가 필요합니다.**
> 키 없이 빌드하면 `.app.tar.gz.sig`가 안 생기고 업데이트 manifest를 만들 수 없습니다.

```sh
# 사전: bun, rustup(aarch64-apple-darwin / x86_64-apple-darwin)
#       서명 키: ~/.tauri/cys-updater.key (최초 1회 `bun x @tauri-apps/cli signer generate`로 생성, 분실 시 자동업데이트 영구 불가)
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/cys-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""   # 키에 암호를 걸었다면 그 값

bun x @tauri-apps/cli build
#  → target/release/bundle/dmg/cys_0.2.0_aarch64.dmg
#  → target/release/bundle/macos/cys.app             (cysd·cys externalBin 동봉)
#  → target/release/bundle/macos/cys.app.tar.gz(.sig) (자동 업데이트용 — 서명 키 있을 때만)

# 배포본으로 정리 (아키텍처 접미사 표준화)
cp target/release/bundle/dmg/cys_0.2.0_aarch64.dmg dist-mac/cys-0.2.0-macos-arm64.dmg

# 업데이트 manifest(latest.json) + 자산 생성
sh scripts/make-update-manifest.sh 0.2.0 <OWNER> cys-terminal
#  → dist-update/latest.json, dist-update/cys-0.2.0-macos-aarch64.app.tar.gz
```

`beforeBuildCommand`(scripts/bundle-prep.sh)가 UI 번들 + cys/cysd 릴리스 빌드 + `externalBin` 배치를
자동 수행합니다. Intel 빌드가 필요하면 `--target x86_64-apple-darwin` 추가(manifest의 `darwin-x86_64`에 키 추가).

### ★Apple 서명·공증 (다른 맥 배포의 유일한 정공법 — 2026-06-15)

**왜 필수인가**: ad-hoc 서명 빌드는 *빌드한 맥*에선 우클릭→열기로 되지만, **다른 맥으로
전송하면** 파일에 `com.apple.quarantine`가 붙고 macOS(Sequoia+)가 **ad-hoc·미공증 앱을
"손상됨"으로 차단**한다(실측 2026-06-15: `spctl -a`=rejected). 공증해야만 어떤 맥에서도
경고/손상됨 없이 열린다.

**1회 셋업 (사람 단계)**:
1. **Apple Developer Program 가입**($99/년, developer.apple.com)
2. **Developer ID Application 인증서** 발급 → Keychain 설치
   (Xcode > Settings > Accounts > Manage Certificates > + > Developer ID Application,
    또는 developer.apple.com > Certificates)
3. **notarytool 자격증명** — 둘 중 하나:
   - app-specific password: appleid.apple.com > 로그인 및 보안 > 앱 암호 생성
   - 또는 App Store Connect API key(.p8 + Key ID + Issuer ID)
4. **Team ID** 확인: developer.apple.com > Membership

**빌드 (자격증명 env + 헬퍼 스크립트가 자동 codesign+공증+staple+검증)**:
```sh
export APPLE_SIGNING_IDENTITY="Developer ID Application: NAME (TEAMID)"
export APPLE_ID="you@example.com" APPLE_PASSWORD="xxxx-xxxx-xxxx-xxxx" APPLE_TEAM_ID="TEAMID"
#   (또는 API key: APPLE_API_KEY_PATH=…/AuthKey_XXXX.p8 APPLE_API_KEY=KEYID APPLE_API_ISSUER=ISSUER)
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/cys-updater.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""

bash scripts/build-macos-signed.sh  # env 검증 → tauri build(자동 공증) → spctl/stapler 검증 → dist-mac + manifest
#  (반드시 bash — 스크립트가 프로세스 치환 `< <(...)`(bash 전용)을 쓴다. `sh`로 실행하면 line 57 syntax error.)
```
- 배선: `tauri.conf.json > bundle.macOS.entitlements = entitlements.plist`(hardened runtime +
  사이드카 cysd·cys 로드 허용). Tauri가 빌드 중 Developer ID codesign + notarytool 제출 +
  staple 을 자동 수행한다(별도 `codesign`/`notarytool` 수동 호출 불요).
- **검증 통과 기준**: `spctl -a -vv cys.app` = **accepted**. (rejected면 공증 실패 — 빌드
  로그의 notarization 결과 확인.)
- 공증 빌드는 **ad-hoc 재서명·`xattr` 우회가 전혀 불필요**하다.

> 인증서가 없을 때(개발용): env 없이 `bun x @tauri-apps/cli build` → ad-hoc 빌드. 이 빌드는
> **다른 맥 전송 시 "손상됨"**이 뜨므로, 받은 맥에서 `xattr -dr com.apple.quarantine
> /Applications/cys.app` 로만 우회 가능(배포용 아님).

### ★비기술자(청중) 배포 전 게이트 체크리스트 (D6 제품 모드)
오너 대표 산출물을 제3자에게 패키징해 내보내기 전, 아래를 **모두** 확인한다.
- [ ] **공증 빌드**(`spctl -a -vv cys.app` = accepted) — 미공증은 비기술자 배포 금지(다른 맥에서 "손상됨" 차단).
- [ ] **신뢰선 라벨 활성** — 스킬 보드 산출물에 "🔒 AI 보조 생성 · 오너 검수 전"이 부착되는지(과대약속 "80~90%" 금지).
- [ ] **외부발행은 master 승인 경유** — 제3자 공유/전송은 자율주행 denylist의 "외부발행(비가역)"에 해당. `cys feed push --wait`(master 승인)를 거친다. 임의 전송 금지(§4 외부발행 원칙 계승).
- [ ] **HITL 미리보기 보존** — 제품 모드도 입력 모달·validate_ir 게이트·미리보기 확인을 우회하지 않는다("1클릭"이라도 게이트 제거는 REJECT).
- [ ] **청중 프로파일 확인** — `~/.cys/profile.json` audience가 대상 청중과 일치(민감 스킬은 카탈로그 미포함=암묵 차단).

## 2. [LEGACY·폐기] Windows 수동 빌드 (MSI + ZIP) — 현행은 CI NSIS, 따르지 말 것

> Windows 머신(또는 Parallels Win11 ARM64)에서 수행. 코어는 검증 완료.

```powershell
# 사전: rustup target add x86_64-pc-windows-msvc aarch64-pc-windows-msvc
cargo build --release --bin cys --bin cysd --target x86_64-pc-windows-msvc
cargo build --release --bin cys --bin cysd --target aarch64-pc-windows-msvc

# WiX(candle/light)로 MSI 생성 — dist-win/cys.wxs(arm64)·cys-x64.wxs(x64) 사용
#   ProgramFiles에 cys.exe·cysd.exe 설치 + PATH 등록
candle dist-win\cys-x64.wxs -o cys-x64.wixobj
light  cys-x64.wixobj -o dist-win\cys-0.2.0-windows-x64.msi
candle dist-win\cys.wxs    -o cys.wixobj
light  cys.wixobj    -o dist-win\cys-0.2.0-windows-arm64.msi

# ZIP (설치 없이)
Compress-Archive target\x86_64-pc-windows-msvc\release\cys.exe,cysd.exe `
  dist-win\cys-0.2.0-windows-x64.zip
```

GUI 앱의 Windows Tauri 빌드는 잔여 — 현재 Windows는 CLI+데몬 중심 배포.

### ★macOS에서 Windows 크로스빌드 (Windows 머신 없이 — 2026-06-15 실증)

Windows 머신이 없어도 macOS에서 MSI까지 만들 수 있다. **windows-gnu 타깃**(wxs Source가
가리키는 `x86_64-pc-windows-gnu`·`aarch64-pc-windows-gnullvm`)을 zig 링커로 크로스컴파일하고,
WiX 대신 **msitools(wixl)**로 MSI를 만든다. (cys.wxs는 표준 WiX v3라 wixl이 그대로 읽는다.)

```sh
# 사전: rustup(homebrew rust와 별개) · cargo-zigbuild · zig · msitools(wixl)
#   brew install zig msitools && cargo install cargo-zigbuild
#   curl --proto '=https' -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
rustup target add x86_64-pc-windows-gnu aarch64-pc-windows-gnullvm

# 바이너리 크로스컴파일 (GUI 없이 cys+cysd만)
cargo zigbuild --release --target x86_64-pc-windows-gnu      --bin cys --bin cysd
cargo zigbuild --release --target aarch64-pc-windows-gnullvm --bin cys --bin cysd

# MSI (wixl — wxs Source 상대경로가 ../target/... 이므로 dist-win에서 실행)
cd dist-win
wixl -o cys-0.2.1-windows-x64.msi   cys-x64.wxs
wixl -o cys-0.2.1-windows-arm64.msi cys.wxs
cd ..
# ZIP
zip -j dist-win/cys-0.2.1-windows-x64.zip   target/x86_64-pc-windows-gnu/release/cys.exe target/x86_64-pc-windows-gnu/release/cysd.exe
zip -j dist-win/cys-0.2.1-windows-arm64.zip target/aarch64-pc-windows-gnullvm/release/cys.exe target/aarch64-pc-windows-gnullvm/release/cysd.exe
```

⚠ **한계(정직)**: 크로스빌드 산출물은 PE 포맷·아키텍처는 검증되나(`file`로 PE32+ x86-64 /
Aarch64 확인) **실제 Windows에서 실행 검증은 불가**하다. 광범위 배포 전 Windows 머신에서
스모크테스트(설치→`cys status`) 권장.

## 3. GitHub 저장소 최초 설정 (1회)

자동 업데이트의 endpoint가 GitHub Releases이므로 **공개 repo가 있어야** 작동합니다.

```sh
# 1) GitHub에 공개 repo 생성 (이름은 cys-terminal 권장 — endpoint와 일치)
gh repo create <OWNER>/cys-terminal --public --source . --remote origin

# 2) tauri.conf.json의 updater.endpoints에서 OWNER를 실제 GitHub 사용자명으로 치환
#    "https://github.com/<OWNER>/cys-terminal/releases/latest/download/latest.json"
#    → 치환 후 반드시 앱을 다시 빌드해야 새 endpoint가 번들에 박힌다.

git push -u origin main
```

## 4. GitHub 릴리스

`latest.json`을 **항상 최신 릴리스에 포함**해야 updater가 찾습니다(endpoint가 `/releases/latest/`).

```sh
# 태그
git tag -a v0.2.0 -m "cys 0.2.0 — 자비스 네이티브 기능 19건 + zero-setup 온보딩 + 자동 업데이트"

# gh CLI 릴리스 (드래프트로 먼저 검토 권장)
gh release create v0.2.0 --draft --title "cys 0.2.0" --notes-file docs/RELEASE_NOTES_0.2.0.md \
  dist-update/latest.json \
  dist-update/cys-0.2.0-macos-aarch64.app.tar.gz \
  dist-mac/cys-0.2.0-macos-arm64.dmg \
  dist-win/cys-0.2.0-windows-x64.msi \
  dist-win/cys-0.2.0-windows-arm64.msi \
  dist-win/cys-0.2.0-windows-x64.zip
```

### 자동 업데이트 동작 요약 (사용자 입장)
- 앱이 시작 시 + 6시간마다 `latest.json`을 조용히 확인 → 새 버전이면 상단 **Update** 버튼에 `!` 배지.
- 버튼 클릭 → 세션이 0개면 자동 설치, 세션이 있으면 "N개 종료됩니다" 확인 후 설치.
- 설치 = 새 `.app` 교체 + 구 데몬 SIGTERM + 앱 재시작(새 cysd 자동 기동). **재설치 불필요.**

⚠ **`git push`·`gh release`·`gh repo create`는 외부 발행(비가역)** — 오너 명시 승인 후에만 실행.
본 문서의 명령은 절차 기록일 뿐, 에이전트가 임의 실행하지 않는다.

## 5. 서명 키 백업 (중요)

`~/.tauri/cys-updater.key`(private)를 **분실하면 이후 버전에 서명할 수 없어 자동 업데이트가 영구 중단**됩니다.
- 안전한 곳(암호 관리자·오프라인 백업)에 보관. git에 절대 커밋 금지.
- 공개키(`tauri.conf.json`의 `pubkey`)는 이미 사용자 앱에 박혀 있어, 같은 private 키로만 새 업데이트를 서명할 수 있습니다.

## 4. 릴리스 전 체크리스트

- [ ] `cargo build --release` 무오류 · `cargo clippy --bins` 0경고 · `cargo test` 통과
- [ ] 신규 머신 시뮬레이션: 빈 HOME에서 `cys list` → 데몬 자동기동 + pack 자동설치 확인
- [ ] DMG에서 설치 → 앱 실행 → `cys status` 동작
- [ ] 버전 문자열 4곳(+wxs 2곳) 일치
- [ ] 릴리스 노트(RELEASE_NOTES_0.2.0.md) 작성
