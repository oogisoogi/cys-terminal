# Pack 전체통합 (옵션3+4) 구현 계획 — DMG 자기완결 패킹

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development or executing-plans. Steps use `- [ ]`.

**Goal:** cysjavis-pack의 **git-추적 전체 트리**(개인정보 gitignore 제외)가 ⓐ cys/cysd 바이너리에 자동 임베드(옵션3)되고 ⓑ 서명된 pack.tar.gz가 .app `Contents/Resources/`에 동봉(옵션4)되어, DMG가 오프라인 자기완결 + 가시·갱신가능해지고, 박사님이 추가하는 **모든** 파일(스킬 아닌 것 포함)이 재빌드 시 자동 통합된다.

**Architecture:** ① build.rs를 `cysjavis-pack/skills/`만 워크하던 것에서 **전체 트리**로 일반화하되 소스를 **`git ls-files cysjavis-pack`(추적전용)**로 잡아 gitignore(개인정보) 경계를 구조적으로 강제 + **hard count floor**(임베드 <250이면 build panic)로 비-hermetic 빈-pack 출하 차단 + git 부재 시 hard-fail. 수동 PACK 84-include_str! 테이블 폐지(자동생성으로 흡수). ② 콘텐츠 스캔 hard-gate(홈경로·이메일·키 — 계정핸들 허용). ③ pack.tar.gz를 bundle.resources로 .app 동봉.

**박사님 결정 반영:** 옵션3+4 둘 다 · ysfuture는 기본값만 제너릭화(아키텍처 식별자·테스트픽스처 보존, 콘텐츠 스캔은 핸들 허용) · 개인정보 즉시 교정.

**적대검증 가드(필수 — 구현 전 인지):**
- 🔴 CRITICAL: `git ls-files` 소싱은 git 인덱스 밖에서 빈 목록 → 빈 pack 출하. **완화 = hard count floor(panic if < 250) + git 부재/빈목록 시 build.rs panic(loud fail).**
- 🟠 HIGH: 콘텐츠 누출은 gitignore가 못 잡음 → **콘텐츠 스캔을 발행 hard-gate로(부수 아님)**, 바이너리 link 전 + pack.tar.gz 발행 전 2지점.
- 🟠 HIGH: CI 역방향 게이트가 build.rs 제외규칙을 python으로 재구현하면 드리프트 → **단일 SOT(`cys pack-manifest` 출력)와 대조**, 재유도 금지.
- 🟡 MEDIUM: capability_catalog.json은 git-추적이라 이제 자동 임베드됨(누락 해소). 재생성 파생물이라 스테일 가능 — 알려진 경미 이슈로 문서화(부트 시 javis_registry.py build가 갱신).
- 🟡 cargo clean 강제(스테일 생성표 회귀 차단).

---

## File Structure

| 파일 | 책임 | 작업 |
|---|---|---|
| `build.rs` | skills-only 워크 → **git ls-files 전체트리** 임베드 테이블 생성 + hard floor + no-git fail | Modify |
| `src/pack.rs` | 수동 PACK 84 include_str! 테이블 폐지, 생성 테이블만 include! | Modify |
| `scripts/scan-pack-secrets.sh` | 콘텐츠 스캔 hard-gate(홈경로·이메일·키; placeholder·핸들 allowlist) | Create |
| `scripts/bundle-prep.sh` | pack.tar.gz 생성 + bundle.resources 위치 배치 | Modify |
| `src-tauri/tauri.conf.json` | bundle.resources에 pack.tar.gz(+manifest+minisig) 추가 | Modify |
| `.github/workflows/release.yml` | 역방향 커버리지 게이트 + 콘텐츠 스캔 게이트 + cargo clean | Modify |
| `docs/INSTALL.md` | .app Resources의 pack.tar.gz·오프라인 자기완결 문서화 | Modify |

---

## Task 1: 콘텐츠 스캔 hard-gate (먼저 — 발행 안전망)

**Files:** Create `scripts/scan-pack-secrets.sh`

- [ ] **Step 1** 스크립트 작성 — `git ls-files cysjavis-pack` 전수에 대해:
  - 차단(비0 종료): `/Users/<실유저>`(`/Users/x`·`/Users/you`·`/Users/NAME` placeholder 제외) · `/home/<user>` · 이메일(`example.com`·`afhi.org`·`noreply`·`anthropic` 제외) · 키/토큰(`sk-…`·`ghp_`·`Bearer …`·`api_key=…`)
  - 허용: 계정 핸들(`ysfuture`·`cysinsight` — 박사님 결정=아키텍처 식별자) · placeholder 경로
  - 발견 시 file:line 출력 + exit 1
- [ ] **Step 2** 실행 검증: `bash scripts/scan-pack-secrets.sh` → 현재 트리(홈경로·ysfuture기본값 교정 후) exit 0 기대. (역회귀 테스트: /Users/cys 한 줄 임시 추가 → exit 1 확인 후 되돌림)
- [ ] **Step 3** 커밋

## Task 2: build.rs 전체트리 임베드 일반화 (옵션3 코어)

**Files:** Modify `build.rs`, `src/pack.rs`

- [ ] **Step 1** build.rs: `base="cysjavis-pack/skills"` 워크를 **`git ls-files cysjavis-pack`** 소싱으로 교체. 각 추적파일 `(rel, include_str!(concat!(CARGO_MANIFEST_DIR,"/",path)))` 엔트리를 `OUT_DIR/pack_all.rs`(PACK_ALL)로 생성. **가드**: ① git 명령 실패/빈출력 → `panic!("pack 소스 비었음 — git 인덱스 부재? 빌드 중단")` ② 생성 엔트리 < 250 → `panic!`. `cargo:rerun-if-changed=cysjavis-pack` 유지. 기존 skills 전용 `pack_skills.rs`는 PACK_ALL로 흡수(중복 제거).
- [ ] **Step 2** pack.rs: `PACK: &[...]` 수동 84 테이블 삭제, `include!(OUT_DIR/pack_all.rs)`로 PACK_ALL 단일 사용. `PACK.iter().chain(PACK_SKILLS.iter())` 사용처(`:546/:1154/:1407` 등)를 `PACK_ALL.iter()`로 갱신. trusted-keys 키링 상수(build.rs:64-76)는 보존.
- [ ] **Step 3** 검증: `cargo build --release --bin cys` → 임베드 수 ≥ git ls-files cysjavis-pack 수(약 291). `strings target/release/cys | grep -c "round/capability_catalog.json"` ≥1(이전 누락분 이제 포함). `cargo test`.
- [ ] **Step 4** embed-vs-disk 동일성: `cys pack-manifest` 파일수 == `git ls-files cysjavis-pack | grep -v <build.rs 제외규칙>` 수 일치 확인.
- [ ] **Step 5** 커밋

## Task 3: 역방향 커버리지 CI 게이트 (단일 SOT)

**Files:** Modify `.github/workflows/release.yml`

- [ ] **Step 1** pack-artifacts 잡에 스텝 추가: `git ls-files cysjavis-pack` MINUS (build.rs 제외규칙: tests/·dotfiles) 집합이 **`cys pack-manifest` 출력 manifest.files에 전부 포함**되는지 대조 → 누락(디스크 고아) 시 exit 1. 제외규칙은 python 재구현 금지 — build.rs가 방출하는 실제 테이블(`cys pack-manifest`)을 SOT로.
- [ ] **Step 2** 같은 잡에 `scripts/scan-pack-secrets.sh` 호출(콘텐츠 스캔) — 바이너리 빌드 전 + pack.tar.gz 생성 후 발행 전 2지점. 비0이면 잡 실패(발행 차단).
- [ ] **Step 3** build 잡에 `cargo clean`(또는 타깃 캐시 무효화) 선행 — 스테일 생성표 회귀 차단.
- [ ] **Step 4** 커밋

## Task 4: pack.tar.gz를 .app Resources에 동봉 (옵션4)

**Files:** Modify `scripts/bundle-prep.sh`, `src-tauri/tauri.conf.json`, `docs/INSTALL.md`

- [ ] **Step 1** bundle-prep.sh: 사이드카 cp 후 `cys pack-manifest`→결정론 pack.tar.gz(+manifest+minisig) 생성해 `src-tauri/resources/`에 배치(이미 CI pack-artifacts 로직 재사용). 미서명 로컬 빌드는 minisig 생략 허용.
- [ ] **Step 2** tauri.conf.json bundle에 `"resources": { "resources/pack.tar.gz": "./", "resources/pack-manifest.json": "./" }` 추가 → `Contents/Resources/`에 복사. (★raw 트리 글롭 금지 — 서명된 단일 blob만, 개인정보·쓰레기 박제 회피)
- [ ] **Step 3** (선택) 첫 실행 오프라인 폴백: cysd init이 네트워크 전에 `resource_dir()/pack.tar.gz`를 minisig 검증 후 사용(cys.rs:4814 다운로드 경로에 resource 분기). 바이너리 임베드가 이미 오프라인 충족하므로 P2(가시성·핫스왑 목적).
- [ ] **Step 4** 검증: 빌드 후 `cys.app/Contents/Resources/`에 pack.tar.gz 존재 + `tar tzf` 파일수 == 임베드수. codesign 무손상(데이터=CodeResources 봉인).
- [ ] **Step 5** INSTALL.md 문서화 + 커밋

## Task 5: 보안 잔여 (이미 일부 완료)

- [x] holdout_eval.py /Users/cys → $HOME 상대화 (커밋 9f7bf1c)
- [x] cys-dept ysfuture 기본값 제거 (커밋 b0f786a) — ★박사님 env 설정 필요
- [ ] capability_catalog.json 스테일 메모 문서화(부트 javis_registry.py build 갱신 확인)

---

## 부록 — 적대검증 미해결/주의
- git ls-files 소싱은 **untracked 스킬을 안 실음**(박사님이 git add 안 한 스킬). 의도적(보안 우선 — untracked 개인파일 누출 차단). 대신 hard floor로 빈-pack 방지.
- 빌드 재현성: build.rs git 셸아웃 의존. 완전 결정론 원하면 release 시 pack-files.txt 커밋→build.rs read로 분리(P2 enhancement).
- bundle.resources는 ! 부정글롭 미지원 → 반드시 화이트리스트(서명 tar blob)만.
