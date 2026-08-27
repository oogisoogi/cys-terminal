# cys v0.14.27 rebase + 로컬 빌드·서명 보고서

- **TICKET**: `cys-fork-rebase-v0.14.27`
- **작업자**: worker-cys@surface:547 · 2026-08-28 (KST)
- **저장소**: `~/cys-terminal-src` · 새 브랜치 `rebase/v0.14.27`(구 `rebase/v0.14.10` 보존)
- **절차 정본**: `~/axdev/master/CYS-UPDATE-POLICY.md` §0(벤더 직접 설치 금지 · 유일 경로 = 포크 리베이스)
- **형식 승계**: `docs/rebase-v0.14.10-report-2026-08-02.md`(직전 리베이스 보고서)

---

## 0. 한 줄 결론

upstream **186커밋**(v0.14.10 → v0.14.27)을 편입하고 그 위에 우리 커밋 **24개를 전부 되살렸다**.
스킵 0 · 누락 0. 충돌 8곳 중 7곳은 판정 여지가 없었고, **판정이 갈린 1곳은 master 에 상신해 C안을
받았다**(▶CEO·▶부서장 버튼 복원). 테스트·타입 게이트는 **신규 실패 0건**이다.

---

## 1. 완료 기준 대비 결과 (티켓 §1 · 전부 실측)

| # | 완료 기준 | 결과 | 근거(실행한 명령·출력) |
|---|---|---|---|
| 1 | 브랜치 `rebase/v0.14.27` = v0.14.27 위 우리 24커밋 · origin push | ✅ **충족** | `git rev-list --count v0.14.27..HEAD` → `27`(원본 24 + §3 판정 반영 1 + §2 이월 1 + 타입게이트 봉합 1). push 후 실측 `git ls-remote origin rebase/v0.14.27` → `aca2c2916a476f75608c9537edeaee4741b56a7c` = 로컬 HEAD 와 동일. 2단계 핸드셰이크로만 실행(§8) |
| 2 | `cys.app` 빌드 · `CFBundleShortVersionString` = `0.14.27` | ✅ **충족** | `BUILD_EXIT=0` · `plutil -extract CFBundleShortVersionString raw …/Info.plist` → **0.14.27**. §5 |
| 3 | 테스트 = 신규 실패 0건 | ✅ **충족** | rust 1430 passed / 0 failed(기준선 1392/0) · ui 810 pass / 0 fail(기준선 706/0) · typecheck exit 0(기준선 exit 0). §4 |
| 4 | 서명 `cys-local` + `codesign -dv` · `spctl` | ✅ **충족** | `Authority=cys-local` · `--verify --deep --strict` = `valid on disk` exit 0 · `spctl` = `rejected` `origin=cys-local`(정상). §6 |
| 5 | 팩 판본 정합 = 0.14.27 | ✅ **충족**(단 설치 시 함정 1건 — §7-1) | 동봉 팩 460파일 · 판본은 `CARGO_PKG_VERSION` 파생 = 0.14.27 · 저장소 `0.15.0` 잔존 0건 |
| 6 | 커스텀 생존 대조표 24행 | ✅ | §2 |
| 7 | 보고서 | ✅ | 이 문서 |

---

## 2. 커스텀 생존 대조표 (24/24 · 스킵 0)

`git checkout -b rebase/v0.14.27 rebase/v0.14.10` → `git rebase --onto v0.14.27 v0.14.10`

원본 커밋과 이식 커밋의 **삽입/삭제 라인수·파일수를 전건 대조**했다. 22건이 완전 일치(=벤더 변경과
겹치지 않아 그대로 앉았다), 2건이 의도된 차이이며 그 차이는 아래에 라인 단위로 규명한다.

| # | 원본 → 이식 | 기능(무엇을 지키는 커밋인가) | 닿는 파일 | 원본 | 이식 | 판정 |
|---|---|---|---|---|---|---|
| 1 | `00dc54c`→`48c8877` | 페인 제목 폰트 통일·확대 + 역할점을 작동 중에만 깜빡 | appearance.ts · main.ts · style.css | 26+/9- | 26+/9- | 동일 |
| 2 | `59579b6`→`c48c3ba` | 영역별 폰트 커스터마이징(제목/본문/메뉴 크기·굵기 · 제목색=역할색) | main.ts · style.css | 54+/5- | 54+/5- | 동일 |
| 3 | `9df052d`→`61a8604` | 제목 앞짤림 수정 · usage 제거 · UI 크롬 확대 · font-smoothing | style.css | 8+/6- | 8+/6- | 동일 |
| 4 | `8b6827a`→`adaa8c1` | 리뷰어 역할점 색 통일 + 점 8→10px | appearance.ts · style.css | 2+/2- | 2+/2- | 동일 |
| 5 | `5a2e20e`→`ade617c` | 정렬 버튼을 최좌측으로(닫기와 분리 — 오클릭 방지) | index.html | 1+/1- | 1+/1- | 동일 |
| 6 | `fc359eb`→`bc0be40` | 역할점 영구 깜빡 수정(stale 자기보고 불신 · 판정 데이터원 교체) | appearance.test.ts · appearance.ts · main.ts | 74+/22- | 74+/22- | 동일 |
| 7 | `7758d87`→`e65ae2b` | 정렬 커스텀 레이아웃 — 4열 균등(master·CSO·워커·리뷰어) | index.html · main.ts | 24+/30- | 24+/30- | 동일 |
| 8 | `989b314`→`eb57363` | 폰트 굵기 4단계 → 100~800 8단계(variable 대응) | main.ts | 7+/1- | 7+/1- | 동일 |
| 9 | `0fa197f`→`49a2077` | dev 빌드 정체성 분리 — 도크 유령 타일 제거 | tauri.dev.conf.json · docs | 45+/0- | 45+/0- | 동일 |
| 10 | `d1718aa`→`d64f64a` | 레이아웃 영구화 재설계 초안 | docs | 152+/0- | 152+/0- | 동일 |
| 11 | `0f84873`→`385cdba` | 레이아웃 persist 설계 v2(codex BLOCK 9지적 반영) | docs | 136+/112- | 136+/112- | 동일 |
| 12 | `307cac5`→`c07dc25` | 레이아웃 설계 v3(phasing·검토주체) | docs | 49+/21- | 49+/21- | 동일 |
| 13 | `977fb6b`→`4b9bb65` | 정렬 = 개수 무관 1행 가로 균등(역할별 열 묶음 폐기) | index.html · main.ts | 15+/36- | 15+/36- | 동일 |
| 14 | `5724d58`→`19768b6` | **사용량 사이드바** + 모델만 남긴 statusline + 메뉴 배율(신규 모듈 `wsusage.ts`) | cys.rs · index.html · appearance{,.test}.ts · main.ts · style.css · wsusage{,.test}.ts | 1093+/40- (13파일) | 1087+/34- (8파일) | ★차이 — 아래 §2-A |
| 15 | `26e8412`→`8367dcd` | 사용량 원천 = 계정 저장소 · Fable 필드경로 · 모델을 제목으로 · named CTX | tauri main.rs · cys.rs · handlers/main/named/panetitle/state.rs · main.ts · style.css · wsusage{,.test}.ts | 1034+/57- | 1034+/57- | 동일 |
| 16 | `7091268`→`2c4240d` | OAuth usage API 편입 — Fable 주간 실게이지 · codex 행 비표시 | tauri main.rs · accounts.rs · cysd main.rs · main.ts · style.css · wsusage{,.test}.ts | 669+/6- | 669+/6- | 동일 |
| 17 | `6fb07c3`→`cc86817` | Fable 자체 집계 줄 삭제 · named CTX 디스크 지속 | accounts/handlers/named/state.rs · main.ts · style.css · wsusage{,.test}.ts | 462+/215- | 462+/215- | 동일 |
| 18 | `3dc703f`→`b1b0d5c` | 페인 CTX 서열 = master → cso → 이름 → 번호 | wsusage{,.test}.ts | 59+/4- | 59+/4- | 동일 |
| 19 | `d2e2beb`→`b4bf3b6` | **운영 마찰 3결함** — 정지 scrollback · 침묵 발신거부 · 종료 후 role 잔존 | cys.rs · governance.rs · handlers.rs · state.rs | 751+/30- | 716+/30- | ★차이 — 아래 §2-B |
| 20 | `4475f28`→`7a9c2b2` | 페인 CTX 행별 관측 나이 병기 · 푸터 「갱신」→「가장 낡음」 | main.ts · style.css · wsbar{,.test}.ts · wsusage{,.test}.ts | 183+/5- | 183+/5- | 동일 |
| 21 | `c48dbdf`→`ac8b6d9` | 사이드바 사용량 패널 글자 20px 고정(두 배율 비연동) | main.ts · style.css · wsbar{,.test}.ts | 96+/28- | 96+/28- | 동일 |
| 22 | `5930ddb`→`84739e4` | 사이드바 글자 배율 하나로 헤더·목록·사용량 패널 동시 조절 | index.html · main.ts · style.css · wsbar{,.test}.ts | 245+/76- | 245+/76- | 동일 |
| 23 | `27fa716`→`f0a91d0` | 사이드바 기준 크기 서열 교정(목록 제목 > 상단 버튼 = 사용량 패널) | style.css · wsbar{,.test}.ts | 150+/32- | 150+/32- | 동일 |
| 24 | `a70b0d7`→`d396327` | 알약 버튼 3종 한 단계 축소 — ▶CEO·▶부서장·＋부서 16→14px(오너 실기기 판정) | style.css · wsbar.test.ts | 53+/2- | 53+/2- | 동일 |

### 2-A. #14 의 차이 — 판본 문자열 5파일이 빠진 것이 전부다

원본은 **13파일**, 이식본은 **8파일**. 빠진 5개는 전부 「0.14.10 → 0.15.0」 한 줄짜리 판본 승격이다.

| 빠진 파일 | 원본 numstat |
|---|---|
| `Cargo.lock` | 2+/2- |
| `Cargo.toml` | 1+/1- |
| `src-tauri/Cargo.toml` | 1+/1- |
| `src-tauri/tauri.conf.json` | 1+/1- |
| `ui/package.json` | 1+/1- |
| 합계 | **6+/6-** |

1093−1087 = **6**, 40−34 = **6** ⇒ 차이가 정확히 이 5파일이고 그 밖은 없다. 기능 파일 8개는
`21+/27-`(cys.rs) 부터 `310+/0-`(wsusage.ts) 까지 **한 줄도 어긋나지 않았다**(위 표의 파일별 대조).
판본은 박사님 결정 ①대로 **0.14.27** 이다 — 0.15.0 라벨은 폐기했고, 저장소 전체에 `0.15.0`
잔존 문자열은 **0건**(`grep -rn '0\.15\.0'` on rs/toml/json/ts/html, target·node_modules 제외).

### 2-B. #19 의 차이 — 우리 ⑵ 와 벤더 B3 가 **같은 결함을 각자 고쳤다**

`src/bin/cys.rs` 만 `100+/7-` → `65+/7-`(−35). 나머지 3파일(`governance.rs 224+/0-` ·
`handlers.rs 325+/23-` · `state.rs 102+/0-`)은 **완전 일치**한다.

무슨 일이 있었나: 우리 커밋 ⑵(2026-08-07 · 타이핑 가드에 막힌 `cys send` 본문이 소실되던 결함)와
upstream **B3(0.14.24)** 가 같은 자리를 각자 고쳤다. 두 수리의 관계는 겹침이 아니라 **직렬**이다.

- 우리 ⑵ 의 고유분 = **기다리기**. 가드는 3초짜리 창이므로, 즉시 포기하지 말고 창이 닫힐 때까지
  0.7초 간격으로 직접 전송을 재시도한다(`send_guard_wait_secs()` · 기본 6초 · `CYS_SEND_GUARD_WAIT_SECS`).
  가드를 우회하지 않는다 — 사람이 계속 치면 계속 거부된다.
- 벤더 B3 의 고유분 = **순수 술어로 분리한 큐 전환**(`should_queue_fallback_send(queued, clear_first, err)`),
  그리고 전환 직후의 `warn_if_daemon_paused()` 고지.

이식본은 **둘을 직렬로 합쳤다**: 기다린다(우리) → 그래도 막히면 술어로 판정해 큐로 1회 전환한다(벤더).
버려진 35줄은 우리 쪽의 **중복 조각**이다 — 벤더가 이미 루프 밖에 만들어 둔 `let body` / `let tag`
재선언 2줄과, 벤더의 술어 분기와 같은 일을 하는 우리 인라인 폴백 블록. 즉 **기능이 빠진 것이 아니라
같은 기능의 두 번째 사본이 빠졌다.**

`clear_first` 취급만 벤더 쪽으로 넘어갔다: 우리 원본은 `clear_first` 를 떨어뜨리며 큐로 전환했고,
벤더 술어는 그 조합을 아예 전환 대상에서 뺀다(큐+clear_first 는 데몬이 `invalid_params` 로 거부하므로
폴백이 두 번째 오류가 된다). 벤더 쪽이 더 보수적이고, 그 보수성을 벤더 자신의 단위 테스트가 지키고
있어 그대로 뒀다.

**생존 실측**: 우리 ⑵ 가 새로 만든 테스트 16건(`role_release_requires_sustained_death` ·
`read_text_lines_serves_grid_when_scrollback_frozen` · `rejected_send_publishes_reason` ·
`tui_redraw_pane_is_stale` 등)은 **전건 현재 트리에 실재**하고 전부 초록이다(§4).

---

## 3. 충돌 8곳 — 해소 내역과 근거

충돌은 **3커밋 · 8지점**에서 났다(나머지 21커밋은 무충돌 자동 적용). 티켓 §3 원칙 —
「같은 파일 다른 줄 = 둘 다 · 같은 줄 = 우리 의도 우선하되 벤더 목적 보존」 — 을 지점마다 적용했다.

| # | 커밋 | 파일 | 충돌 내용 | 해소 | 판정 근거 |
|---|---|---|---|---|---|
| 1 | `5724d58` | `Cargo.lock` (2블록) | 0.14.27 ↔ 0.15.0 | **0.14.27** | 박사님 결정 ①(0.15.0 라벨 폐기) |
| 2 | `5724d58` | `Cargo.toml` | 동상 | **0.14.27** | 동상 |
| 3 | `5724d58` | `src-tauri/Cargo.toml` | 동상 | **0.14.27** | 동상 |
| 4 | `5724d58` | `src-tauri/tauri.conf.json` | 동상 | **0.14.27** | 동상 |
| 5 | `5724d58` | `ui/package.json` | 동상 | **0.14.27** | 동상 |
| 6 | `d2e2beb` | `src/bin/cys.rs` `is_typing_guard_err` 직후 | 벤더 B3 신규 함수 3개 ↔ 우리 `send_guard_wait_secs()` 가 **같은 자리에 추가** | **둘 다** | 다른 줄에 각자 추가된 별개 함수다(§3 「다른 줄 = 둘 다」) |
| 7 | `d2e2beb` | `src/bin/cys.rs` `Command::Send` 본문 | 같은 결함의 두 수리가 **같은 줄**에서 부딪힘 | **직렬 합성**(기다리기=우리 → 술어 큐전환=벤더) | §2-B. 기능 손실 0 · 중복 사본만 제거 |
| 8 | `5930ddb` | `ui/index.html` `#wsbar-head` | 우리=「워크스페이스」 라벨 삭제 ↔ 벤더=▶CEO·▶부서장 버튼 삭제 | **판정 상신 → master C안** | 아래 §3-A |

### 3-A. 판정 1건 — ▶CEO·▶부서장 버튼 (master 판정 C 채택)

이 한 곳만 워커가 단독으로 정할 수 없었다. 티켓 §3 말미(「판단이 갈리는 충돌 = 자율 결정하지 말고
상신 후 계속 진행」)대로 인박스에 올렸고, master 가 **C**를 채택했다(재승인 원장 대조 통과 —
nonce `8391ac` · 대상 `surface:547` · `submitted=yes` · 내 상신 이후 시각).

**벤더 의도 (upstream `3685af9` · 2026-08-20 · `feat(ui): P2 — ▶CEO·▶부서장·'셸에 cys 설치' 버튼 3종 제거`)**

> 기동 경로를 **이원 경로**(pane 마스터 선언 = role-bootstrap 훅 체인 · `cys launch-agent` ·
> phoenix 복원)로 정합시키고, 사이드바 헤더에서 중복 진입점을 덜어낸다.
> 벤더 자신이 코드 주석에 이렇게 적어 뒀다: *"Rust 커맨드(start_master 등)는 존치(git log 참조).
> 버튼 복원은 HTML 2줄+핸들러 재추가로 가역."* 그리고 *"▶CEO/▶부서장 두 버튼은 복원 대상이
> 아니다 — 위 이원 경로가 정본이다."*

**우리 채택 이유 (C = 복원 + 툴팁 1줄)**

1. 이 티켓의 최상위 전제가 **「업데이트해도 커스터마이징은 초기화되지 않는다」**(박사님 절대규칙)인데,
   그 두 버튼은 박사님이 **2026-08-11 실기기에서 크기를 판정한 대상**이다(커밋 `a70b0d7` — 알약 3종
   16→14px). 벤더 삭제를 그대로 따르면 오늘 아침 06:34 벤더 dmg 직접 설치로 커스텀이 사라졌던 그
   사고를, 리베이스라는 이름으로 되풀이하는 것이 된다.
2. 벤더도 **백엔드를 죽이지 않았다**: `src-tauri/src/main.rs` 에 `async fn start_master` ·
   `async fn start_dept_master` · `invoke_handler` 등록이 v0.14.27 에 살아 있고, 「`start_dept_master`
   소실」을 막는 **벤더 자신의 가드 테스트**까지 있다. 즉 벤더가 정리한 것은 기능이 아니라
   진입점이므로, 진입점을 유지해도 벤더 목적과 충돌하지 않는다.
   줄번호는 기준을 밝혀 적는다 — **v0.14.27 원본**: 4424 · 4870 · 5743~5744 · 8190행
   (`git show v0.14.27:src-tauri/src/main.rs | grep -n …`). **우리 트리**: 4447 · 4893 · 8214행.
   두 기준의 줄번호를 섞어 적으면 검증자가 엉뚱한 줄을 열게 되므로 병기한다.
   밀린 폭도 실측이다: `git diff --numstat v0.14.27..HEAD -- src-tauri/src/main.rs` → **24+/0-**,
   그래서 오프셋이 앞쪽 두 심볼은 +23(마지막 삽입 지점보다 위) · 가드 테스트는 +24 다.

   ⚠ 계측 이력 정직 기록: 이 절의 줄번호를 처음엔 **우리 트리 기준으로 재 놓고 v0.14.27 기준이라고
   적었다**(상신 push 06:59:36 포함). 파일에서 다시 재어 잡았고 인박스에 [정정]을 append 했다.
   그 [정정] 자체도 밀린 폭을 「224줄」로 잘못 적어 두 번째 [정정]을 냈다 — 224 는 같은 커밋이
   `governance.rs` 에 얹은 양이다. **봉합 라운드가 자기가 정리하는 병을 새로 만든 사례**라 지운다.
3. 벤더가 스스로 적어 둔 **가역 경로**(HTML 2줄 + 핸들러 재추가)를 글자 그대로 썼다. 되살린 것은
   `ui/index.html` 버튼 2개 · `ui/src/main.ts` 클릭 리스너 2개 + `masterDeniedMsg` 헬퍼 1개뿐이고,
   **백엔드는 한 줄도 건드리지 않았다.** 벤더의 P2 주석도 지우지 않고 그 아래에 포크 판정 주석을
   덧붙였다(벤더 의도 기록 보존).
4. C안의 「+1줄」 = 두 버튼 `title` 끝에 **「대체 경로: pane 마스터 선언·cys launch-agent 로도 기동할 수
   있습니다.」** 를 병기. 벤더의 이원 경로 취지를 버튼 위에 남긴다.

**이 판정이 테스트로 드러난 경위(중요)**: 상신 시점의 잠정 해소(=벤더 삭제 채택)에서 `bun test` 가
**809 pass / 1 fail** 이었고, 그 유일한 적색이 우리 `ui/src/wsbar.test.ts:134`
「버튼은 하나도 잃지 않았다」(expected 6 / received 4)였다. 그 그물이 바로 **박사님 08-11 판정을
고정한 테스트**다. C 반영 후 **테스트를 한 줄도 고치지 않고** 810 pass / 0 fail 이 됐다 —
오너 판정을 테스트에서 지우지 않고 코드로 초록을 만들었다.

### 3-B. 임의 창작 여부

**0건.** 8지점 전부 판정 근거가 실측(판본 결정 · 함수 실재 · 백엔드 존치 · 원장 대조)으로 닫혔고,
근거로 닫히지 않은 1건은 창작하지 않고 상신했다.

---

## 4. 테스트·타입 게이트 — 기준선 대조 (신규 실패 0건)

기준선은 태그를 **별도 worktree**(`~/cys-pristine-v0.14.27`)로 꺼내 `bun install` 까지 한 뒤
같은 명령으로 실측했다(설치 전엔 모듈 미해결 파생 오류가 섞여 대조가 오염된다 — 08-02 선례).

| 축 | pristine v0.14.27 (기준선) | 우리 `rebase/v0.14.27` | 판정 |
|---|---|---|---|
| `cargo test --bins --lib` · lib | 413 passed / 0 failed / 1 ignored | 413 passed / 0 failed / 1 ignored | 동수 |
| 〃 · `src/bin/cys.rs` | 186 passed / 0 failed | **189** passed / 0 failed | +3(우리 것) |
| 〃 · `src/bin/cysd/main.rs` | 793 passed / 0 failed / 1 ignored | **828** passed / 0 failed / 1 ignored | +35(우리 것) |
| 〃 합계 | **1392 / 0 failed** | **1430 / 0 failed** | **신규 실패 0건 · 우리 테스트 38건 순증** |
| `bun test` (ui) | 706 pass / 0 fail (22파일) | **810** pass / 0 fail (23파일) | **신규 실패 0건 · 104건 순증** |
| `bunx tsc -p tsconfig.check.json` | exit 0 · 오류 **0건** | exit 0 · 오류 **0건** | **신규 0건** |

### 4-A. ★08-02 보고서와 달라진 점 — 기준선이 깨끗해졌다

직전 리베이스(0.14.10) 때는 **원본 상태에서도 typecheck 비-테스트 오류 8건**이 나서 "통과"라고
보고할 수 없었고 "베이스라인 동일"로 적었다. v0.14.27 에서는 upstream 이 그 8건을 없앴고,
새로 들어온 `ui/src/bun-env.d.ts`(수기 앰비언트 선언 · 의존성 0)가 테스트 파일 쪽 `TS2307` 까지
메운다. **그래서 이번에는 문구 그대로 "오류 0건"으로 보고할 수 있다.**

그 대신 기준선이 깨끗해지면서, 종전엔 잡음 8건 속에 묻혀 있던 **우리 쪽 1건이 유일한 적색으로
드러났다**: `src/wsbar.test.ts(107,6) TS2339: Property 'each' does not exist on type 'TestFn'`
(우리 커밋 `27fa716` 이 선택자 전수 대조를 `it.each` 표 구동으로 쓴다). 봉합은 `tsconfig.check.json`
이 **스스로 적어 둔 처방**을 따랐다 — *"여기서 에러가 나면 @types 를 설치하지 말고 그 파일에 선언을
추가하라."* `types: []` 라 @types 자동 포함이 꺼져 있어 설치로는 애초에 채워지지 않고, 설치는
`bun.lock` 을 건드려 오프라인 빌드 계약을 깬다. 그래서 런타임 코드 0줄·의존성 0으로 `each` 선언
한 줄만 더했다(커밋 `aca2c29`). upstream 의 `typegate.test.ts`(그 파일의 실존·선언·include 를
못박는 핀) 포함 전건 초록이다.

---

## 5. 빌드

### 명령 (08-02 선례 + 장기기억 `tauri-local-build-traps-cys` 준수)

```bash
unset NODE_OPTIONS                      # NODE_OPTIONS 오염 회피(브리프 지시)
export PATH="$HOME/.cargo/bin:$PATH"    # cargo 는 PATH 에 없고 ~/.cargo/bin 에 있다
bunx @tauri-apps/cli@2 build --bundles app \
     --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

- **updater 서명키 hard-fail 회피**: `createUpdaterArtifacts=true` 라 서명키 없이는 빌드 말미에
  실패한다 → 일회용 키를 만들지 않고 **인라인 config 로 그 항목만 off**.
- **DMG osascript 함정 회피**: `.dmg` 단계는 GUI 권한 없는 에이전트 셸에서 실패한다 →
  **`--bundles app`** 으로 `.app` 만 생성. 설치는 `.app` `ditto` 로 충분하다.
- 귀결: **updater 아티팩트·dmg 부재 = 수동 설치 전용**(의도 · 설치는 master/박사님 몫).

### 결과

```
BUILD_START=2026-08-28T07:05:19+0900
BUILD_END  =2026-08-28T07:06:20+0900      (61초 — target/ 캐시 온난)
BUILD_EXIT =0
Finished 1 bundle at: target/release/bundle/macos/cys.app
```

| 항목 | 값 |
|---|---|
| `Info.plist` `CFBundleShortVersionString` | **0.14.27** ✅(박사님 결정 ① 충족) |
| 사이드카 `cys --version` | `cys 0.14.27` |
| 사이드카 `cysd --version` | `[cysd] v0.14.27 cys-fix-w2-gen-0.14.4` |
| 동봉 팩 | `Contents/Resources/pack.tar.gz` · **2,665,407 B** · **460파일** 결정론 동봉 |

### stale 산출물 오판 차단 (2축)

빌드 **전** 같은 경로에 **0.15.0 / Aug 11 10:49** 산출물이 있었다. 두 축으로 새 빌드임을 확정:

- **버전**: `0.15.0` → **`0.14.27`**
- **mtime**: `Aug 11 10:49:41` → **`Aug 28 07:06:19`**(`Contents/MacOS/cys-app`)

### UI 커스텀 임베드 확인 (`strings` 로는 확인 불가 — 2축 대조)

Tauri 는 프런트엔드를 압축 임베드하므로 바이너리 `strings` 로는 UI 마커가 0건 나온다
(장기기억 `tauri-ui-embed-freshness-verification`). 대신 2축으로 확인했다.

- **축1 — `ui/dist` 안의 우리 커스텀 마커 실재**: `cys-title-size` 3건 · `cys-term-weight` 4건 ·
  `cys-title-color-role` 2건 · `cys-menu-weight` 3건 · `wsbar-font` 4건(+ `dist/style.css` 18건) ·
  `btn-master-start` 1건 · `btn-dept-master` 1건(=§3-A 복원분이 실제로 번들에 들어갔다) ·
  사용량 패널 산출 문자열 `가장 낡음` 1건 · `ws-usage` 1건.
  (모듈 식별자 `wsusage` 자체는 번들러가 이름을 지워 0건이다 — **부재가 아니라 최소화**이므로
  그 모듈이 만드는 문자열로 갈랐다.)
- **축2 — 신선도**: `ui/dist/main.js` `07:05:21` **<** 바이너리 `cys-app` `07:06:19`
  ⇒ **이번 빌드가 이번 dist 를 먹었다.**

---

## 6. 서명 (`cys-local` 고정 서명)

```bash
codesign --force --deep --sign "cys-local" target/release/bundle/macos/cys.app
```

| 항목 | 서명 전 | 서명 후 |
|---|---|---|
| `Identifier` | `cys_app-2c2c82801370d23a` | **`com.cysjavis.terminal`** |
| `Signature` | `adhoc`(링커 기본값 — 우리 서명 아님) | `Authority=cys-local` |
| `Signed Time` | — | **Aug 28, 2026 at 07:08:10** |
| `Sealed Resources` | — | version=2 · rules=13 · **files=4559** |

- `codesign --verify --deep --strict --verbose=2` → **`valid on disk`** · **`satisfies its Designated
  Requirement`** · exit **0**
- `spctl -a -vv` → **`rejected` · `origin=cys-local`** · exit 3 — **이것이 정상이다**(자체 서명은
  Gatekeeper 정책상 거부되며, 티켓 §1-4 가 그렇게 명시한다).
- **TCC 관련 실측**: 새 빌드와 현재 설치본(`/Applications/cys.app` · 0.15.0)이 **같은 Authority
  (`cys-local`) · 같은 Identifier(`com.cysjavis.terminal`)** 다. CDHash 는 당연히 다르다
  (새 빌드 `be3482e8…` · 설치본 `89dc7fc3…`). 고정 서명을 도입한 목적(빌드마다 cdhash 가 바뀌어
  TCC 가 리셋되던 문제)의 **전제 조건은 충족**돼 있다. 다만 실제 TCC 유지 여부는 설치 후에만
  확인되는 값이므로 여기서 단정하지 않는다 — §7-3 후속 확인 항목.

---

## 7. 남은 위험·후속 (판단은 master·박사님 몫)

### 7-1. ⚠ 팩 판본 — 설치해도 새 팩이 **반영되지 않는다**(다운그레이드 차단)

- 빌드가 담는 팩 판본은 `CARGO_PKG_VERSION` 파생이므로 **0.14.27** 이다(별도 판본 파일 없음 —
  `src/pack.rs` 가 `env!("CARGO_PKG_VERSION")` 을 target_version 으로 쓴다). 저장소에 `0.15.0`
  잔존 문자열 **0건**. ⇒ 산출물 5(팩 판본 정합) 자체는 충족.
- **그런데 디스크 팩이 앞서 있다**: `~/.cys/pack/.pack-version` = **`0.15.0`**(현 설치본이 쓴 값).
  `src/pack.rs:2745~2756` 의 비강제 install 은 `디스크 > 바이너리` 면
  `[init-pack] 다운그레이드 차단 — 팩 미반영 (디스크 0.15.0 > 바이너리 0.14.27). 의도적 재설치는 force로.`
  를 찍고 **`(0,0)` 으로 조기 반환**한다. 부트 스윕 게이트(`pack_current_in` · 2147행)도
  `디스크 >= 바이너리` 면 true 라 **스윕을 건너뛴다**.
- **귀결**: 새 앱을 설치해도 **upstream 186커밋이 가져온 팩·디렉티브 갱신이 `~/.cys/pack` 에
  들어가지 않는다.** 앱은 0.14.27, 팩 내용은 0.15.0 시절 그대로가 된다.
- **후속 처리 1줄(master 소관 — 이 티켓 범위 밖)**: 설치 후 **`cys init-pack --force`** 를 1회 집행하면
  차단이 우회된다. user-owned 파일은 `decide_file_action` 이 영구 보존하므로 우리 pack 수정본은
  살아남는다(`.new`/`.user` 병치 + merge-pending 원장). ⚠ 그래도 집행 전 정책 §3 외부 수동 백업이
  선행돼야 롤백 담보가 생긴다.

### 7-2. ⚠ 원격 `rebase/v0.14.10` 이 로컬보다 뒤처져 있다 (무접촉 유지)

`origin/rebase/v0.14.10` = `d2e2beb`, 로컬 = `a70b0d7`(5커밋 앞). 이번 티켓은 그 브랜치를
**건드리지 않았고** 앞으로도 force-push·삭제는 금지다. 다만 원격만 보는 사람에게는 우리 커스텀이
19개로 보인다는 사실을 기록해 둔다 — 정본은 `rebase/v0.14.27`(27커밋)이다.

### 7-3. 설치 후 확인 항목 (08-02 보고서에서 이월 · 여전히 유효)

1. `cys schedule list | grep -i formation` **0건** 확인. 있으면 즉시
   `cys schedule remove formation-ensure-10min`(07-27 실사고 재발 방지).
2. TCC — §6대로 Authority·Identifier 는 같지만 cdhash 는 바뀐다. 페인 안 Claude 의 구글 드라이브
   접근 EPERM 여부를 설치 직후 1회 확인 권고.
3. updater 아티팩트 부재 = 이 빌드는 인앱 업데이트 경로를 갖지 않는다(의도).

### 7-4. 이번에 새로 생긴 것

- 커밋 `64ae329`(§3-A C안)는 **upstream 과 의도가 갈리는 유일한 지점**이다. 다음 리베이스에서
  같은 자리가 다시 충돌할 것이므로, 그때 이 절을 읽고 같은 판정을 반복하거나 재판정하면 된다.
- 커밋 `aca2c29`(§4-A)는 upstream 파일(`ui/src/bun-env.d.ts`)에 우리 선언을 얹은 것이라
  **다음 리베이스에서 충돌 후보**다. 벤더가 같은 선언을 추가하면 우리 것을 버리면 된다.

---

## 8. 금지선 준수 (티켓 §4 대조)

| 금지 항목 | 상태 | 근거 |
|---|---|---|
| `/Applications/cys.app` 설치·교체 | **미실행** | 산출물은 `target/` 안에만 있다. `/Applications` 는 `codesign -dvvv` 로 **읽기만** 했다(§6 대조표) |
| `~/.cys/pack` 수정 · `init-pack` · `pack-merge` | **미실행** | `.pack-version` 을 **읽기만** 했다(§7-1) |
| origin **force-push** | **미실행** | `git push -u origin rebase/v0.14.27`(force 플래그 없음) |
| 기존 브랜치 `rebase/v0.14.10` 삭제·변경 | **미실행** | push 후 실측: 원격 `d2e2beb`·로컬 `a70b0d7` **둘 다 그대로** |
| upstream 에 PR/이슈 | **미실행** | GitHub 이 안내한 PR URL 은 **열지 않았다** |
| 외부 계정·과금 | 해당 없음 | — |

★ **push 는 2단계 핸드셰이크로만 실행했다**(§6-9 불변식 5). ①티켓 지시 → ②「실행 직전 확인 요청」
인박스 push(07:05:48 · 명령·커밋·범위 명시) → ③master 재승인(원장 nonce `b0b67a` · 대상
`surface:547` · `submitted=yes` · 07:05:58 = 내 ② 이후) → ④실행. 판정 1건(§3-A)도 같은 방식으로
원장 대조(nonce `8391ac`)를 거쳤다.

---

## 9. 재현 명령 요약

```bash
cd ~/cys-terminal-src
git log --oneline v0.14.27..rebase/v0.14.27          # 27커밋(원본 24 + 판정 1 + 이월 1 + 타입게이트 1)
git ls-remote origin rebase/v0.14.27                 # aca2c291…

# 기준선(별도 worktree)
git worktree add ~/cys-pristine-v0.14.27 v0.14.27
cd ~/cys-pristine-v0.14.27/ui && unset NODE_OPTIONS && bun install && bun test    # 706 / 0
bunx tsc -p tsconfig.check.json                                                   # exit 0
cd ~/cys-pristine-v0.14.27 && cargo test --bins --lib                             # 413+186+793 / 0

# 우리 트리
cd ~/cys-terminal-src/ui && unset NODE_OPTIONS && bun test                        # 810 / 0
bunx tsc -p tsconfig.check.json                                                   # exit 0
cd .. && export PATH="$HOME/.cargo/bin:$PATH" && cargo test --bins --lib          # 413+189+828 / 0

# 빌드·서명
bunx @tauri-apps/cli@2 build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'
plutil -extract CFBundleShortVersionString raw target/release/bundle/macos/cys.app/Contents/Info.plist  # 0.14.27
codesign --force --deep --sign "cys-local" target/release/bundle/macos/cys.app
codesign --verify --deep --strict --verbose=2 target/release/bundle/macos/cys.app  # valid on disk
spctl -a -vv target/release/bundle/macos/cys.app                                   # rejected · origin=cys-local (정상)
```
