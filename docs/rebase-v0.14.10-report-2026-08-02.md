# cys v0.14.10 rebase + 로컬 빌드 보고서

- **TICKET**: `cys-update-0.14.10-rebase`
- **작업자**: worker@surface:349 · 2026-08-02 (KST)
- **저장소**: `~/cys-terminal-src` · 브랜치 `rebase/v0.14.10`
- **절차 정본**: `01_axdev/master/CYS-UPDATE-POLICY.md` §5 **경로 B(자체 빌드)** · §8 커스터마이징 원칙
- **결론**: rebase 13/13 완주(스킵 0) · 신규 타입오류 0건 · 단위 테스트 215/215 · `cys.app` 0.14.10 빌드 성공

---

## 1. 완료 기준 대비 결과 (전부 실측)

| # | 완료 기준 | 결과 | 근거(실행한 명령·출력) |
|---|---|---|---|
| 1 | rebase 완주 = 13커밋 | ✅ **충족** | `git rev-list --count v0.14.10..rebase/v0.14.10` → `13`. 스킵·누락 0 |
| 2 | ui typecheck 통과 | ⚠ **문구대로는 미충족 — 단 원인은 upstream 선재** | 우리 트리 비-테스트 오류 8건. **pristine v0.14.10 베이스라인도 동일 8건**. 정규화 diff = 완전 일치(신규 0건). §4 상세 |
| 2 | 전체 앱 빌드 | ✅ **충족** | `BUILD_EXIT=0` · `target/release/bundle/macos/cys.app` 생성 |
| 3 | 빌드본 버전 = 0.14.10 | ✅ **충족** | `plutil -extract CFBundleShortVersionString raw …/Info.plist` → `0.14.10` |
| 4 | 금지선 미실행 | ✅ **준수** | push·`/Applications` 설치·cysd/앱 재시작·codesign **전부 미실행**(§6) |
| 5 | 보고서 파일 | ✅ | 이 문서 |

> ★기준 2는 **문구 그대로는 충족할 수 없다.** 이 저장소는 원본 상태에서도 typecheck를 통과하지 못하기
> 때문이다. "통과"로 보고하면 거짓이 되므로 **"신규 타입오류 0건(베이스라인 동일)"** 으로 보고한다.
> 최종 판정은 master 몫이다. 상세·근거는 §4.

---

## 2. 커밋별 결과 (13/13 · 스킵 0 · 변형 0)

`git rebase --onto v0.14.10 v0.13.20 rebase/v0.14.10`

| 순서 | 원 커밋 | 이식 후 | 충돌 | 원본 diff | 이식 diff | 판정 |
|---|---|---|---|---|---|---|
| 1/13 | `a83ec1b` 페인 제목 폰트 통일·확대 + 역할점 작동중에만 깜빡 | `00dc54c` | **1곳** | 26+/9- | 26+/9- | 동일 |
| 2/13 | `9c4c5b9` 영역별 폰트 커스터마이징 | `59579b6` | **1곳** | 54+/5- | 54+/5- | 동일 |
| 3/13 | `bc9c33d` 제목 앞짤림·usage 제거·UI크롬 확대 | `9df052d` | 없음 | 8+/6- | 8+/6- | 동일 |
| 4/13 | `5a87127` reviewer role-dot 색 통일 + 8→10px | `8b6827a` | 없음 | 2+/2- | 2+/2- | 동일 |
| 5/13 | `64b9479` 정렬 버튼 최좌측 이동 | `5a2e20e` | 없음 | 1+/1- | 1+/1- | 동일 |
| 6/13 | `cd78e3b` 역할점 영구 깜빡 수정 | `fc359eb` | **2곳** | 74+/22- | 74+/22- | 동일 |
| 7/13 | `b745668` 정렬 커스텀 레이아웃 4열 균등 | `7758d87` | 없음 | 24+/30- | 24+/30- | 동일 |
| 8/13 | `82fd576` 폰트 굵기 100~800 8단계 | `989b314` | 없음 | 7+/1- | 7+/1- | 동일 |
| 9/13 | `8024ac0` dev 빌드 정체성 분리 | `0fa197f` | 없음 | 45+/0- | 45+/0- | 동일 |
| 10/13 | `d53c336` 레이아웃 영구화 설계 초안 | `d1718aa` | 없음 | 152+/0- | 152+/0- | 동일 |
| 11/13 | `200dc86` 레이아웃 persist 설계 v2 | `0f84873` | 없음 | 136+/112- | 136+/112- | 동일 |
| 12/13 | `8a85c72` 레이아웃 설계 v3 | `307cac5` | 없음 | 49+/21- | 49+/21- | 동일 |
| 13/13 | `7e19cc0` 정렬 = 1행 가로 균등 | `977fb6b` | 없음 | 15+/36- | 15+/36- | 동일 |

- **충돌 커밋 3개 · 충돌 지점 4곳**. 나머지 10커밋은 무충돌 자동 적용.
- **스킵 0 · 변형 0**: 13커밋 전부 원본과 삽입/삭제 라인수가 **완전히 일치**한다(위 표 실측).
- **HEAD** = `977fb6b` · 작업트리 클린(추적 파일 변경 0).

### ★ "충돌을 해소했는데 왜 라인수가 그대로인가" — 검산과 그 해석

충돌 3건에서 `applyDeadAgentState(...)` 호출 줄을 **버렸는데도** 라인수가 원본과 같아 처음엔
계측 오류를 의심했다. 실측으로 규명한 답:

> **그 줄은 우리 커밋이 「추가한 줄」이 아니라 「문맥(context) 줄」이었다.**

`git show a83ec1b -- ui/src/main.ts`의 실제 추가/삭제 줄에는 `applyDeadAgentState`가 **없다**
(해당 줄은 부모 v0.13.20에 이미 있던 upstream CU-6B2 코드이고, 우리 커밋은 그 옆줄을 고쳤을 뿐이다).
문맥 줄을 버리는 것은 **추가 라인수에 영향을 주지 않으므로** 수치 일치는 우연이 아니라 **필연**이다.
이는 브리프 원칙 2가 서술한 상황("upstream이 제거한 서브시스템을 우리 커밋의 문맥 줄이 물고
들어오면 그 줄은 upstream 제거를 따른다")과 **정확히 같은 사례**다.

⚠ 계측 이력 정직 기록: 최초 대조 스크립트는 macOS에 없는 `tac`을 써서 커밋 쌍 짝짓기가
깨졌고 13건 전부를 "★변형"으로 출력했다. 그 출력은 **발견이 아니라 계측 실패**였고,
`tail -r`로 고쳐 재측정한 것이 위 표다.

---

## 3. 충돌 4곳 — 해소 내역과 근거

모든 해소의 공통 판정: **우리 기능 줄만 이식하고, upstream v0.14.10이 통째로 제거한
dead-agent 서브시스템 호출 줄은 부활시키지 않는다**(브리프 원칙 2 · master 판정).

### 판정의 선행 실측 (master 판정을 그대로 믿지 않고 직접 확인)

```
ls ui/src/deadagent.ts                                  → No such file (부재)
grep -rn 'applyDeadAgentState|isDeadAgentPane|deadAgentHeaderText|from "./deadagent"' ui/src/
                                                        → 0건
```
⇒ v0.14.10에서 서브시스템이 **완전 제거**됨을 확인. 되살리면 컴파일 자체가 불가능하다.

| # | 커밋 | 파일:위치 | 충돌 내용 | 해소 |
|---|---|---|---|---|
| 1 | `a83ec1b` | `ui/src/main.ts` refreshPaneTitles | HEAD=`setRoleDot(el, role)` ↔ ours=깜빡 파라미터 + `applyDeadAgentState` | **깜빡 라인만 채택**, dead-agent 라인 폐기 |
| 2 | `9c4c5b9` | `ui/src/main.ts` 동일 루프 | ours 측에 `applyDeadAgentState` + 제목 역할색 라인 | **제목 역할색 라인만 채택** |
| 3 | `cd78e3b` | `ui/src/main.ts` import 블록 | upstream 신규 `routeOnData`(`./mousefilter`) ↔ 우리 `nodeWorking` 추가가 같은 자리에서 충돌 | **양쪽 모두 보존**(둘은 무관한 별개 import) |
| 4 | `cd78e3b` | `ui/src/main.ts` refreshPaneTitles | ours=`surfaceWorking(sid, sk)` 시그니처 변경 + `applyDeadAgentState` | **새 2인자 시그니처 채택**, dead-agent 라인 폐기 |

### 해소 후 정합성 실측 (외과적 변경이 반쪽으로 끝나지 않았는지)

`cd78e3b`은 `surfaceWorking`의 **시그니처를 바꾸는** 커밋이라 호출부 누락이 곧 컴파일 오류다.
전수 확인:

```
ui/src/main.ts:1721  function surfaceWorking(sid: number, socket: string|null|undefined): boolean
ui/src/main.ts:1761  … surfaceWorking(s.surface_id, sk)   ← 충돌 해소 지점
ui/src/main.ts:1776  … surfaceWorking(s.surface_id, sk)   ← 무충돌 자동 적용 지점
ui/src/main.ts:1581  type NodeSig = { … ; working: boolean }      ← 소비처
ui/src/main.ts:2804  working: nodeWorking(n.status, n.idle_secs, n.exited)  ← 생산처
```
⇒ 생산·소비·호출부 전부 일관. 판정 데이터원 교체(lastFleet → nodeSig)가 온전히 이식됐다.

### 임의 창작 여부

**0건.** 브리프 원칙 4(판단 불가 시 중단·질문)에 해당하는 사례는 없었다 — 4곳 모두 판정 근거가
실측(파일 부재·참조 0건·시그니처 전수 대조)으로 닫혔기 때문에 질문 없이 진행했다.

---

## 4. 타입체크 — 베이스라인 대조 (⚠ 기준 2 관련)

### 무엇을 쟀나

`ui/tsconfig.check.json`은 **upstream v0.14.10 소유 파일**이다(`git log v0.14.10 -- ui/tsconfig.check.json`
→ `52b51cc`, 우리 13커밋은 이 파일 **미접촉**). 그 파일이 스스로 적어 둔 재현 명령을 그대로 썼다:

```
cd ui && bunx tsc -p tsconfig.check.json
```

### 결과

| 대상 | 비-테스트 오류 |
|---|---|
| 우리 트리 `rebase/v0.14.10` | **8건** |
| **pristine `v0.14.10`**(별도 worktree 체크아웃 + `bun install`) | **8건** |
| 줄번호 정규화 후 `diff` | **완전 일치 — 신규 0건** |

베이스라인은 태그를 별도 worktree로 꺼내 **deps까지 설치한 뒤** 동일 명령으로 실측했다
(설치 전엔 `@xterm` 모듈 미해결 파생 오류가 섞여 대조가 오염되므로).

### 선재 오류 8건의 정체

- `main.ts` 업데이트 팝오버의 `never` 타입 접근 **7건**(`version`·`notes`·`pack_version`·`manifest_url`·`binary_too_old`)
- `main.ts` `Promise<Workspace>` → `void` 반환 불일치 **1건**

**전부 우리 13커밋이 손대지 않은 영역**이다. 줄번호가 베이스라인 대비 정확히 +36 어긋나 있는데,
이는 우리가 추가한 코드량만큼 아래로 밀린 것으로 오류 자체는 같은 지점이다.

### 집계에서 제외한 것 (제외 사유 명시)

`*.test.ts`의 `Cannot find module 'bun:test'` **15건**은 제외했다. `tsconfig.check.json`이
`"types": []`인 채 `src/**/*.ts`(테스트 포함)를 include하는 **구조적 상수**이고, 베이스라인에도
동일하게 나타나 대조에 의미가 없기 때문이다.

### 단위 테스트

```
cd ui && bun test  →  215 pass / 0 fail (425 expect, 15 files)
```
`cd78e3b`이 복구한 `appearance.test.ts` 계약 테스트(리뷰어색 4종 통일 + `nodeWorking` 4케이스)
포함 전건 통과.

---

## 5. 빌드

### 명령 (07-27 선례 + 장기기억 `tauri-local-build-traps-cys` 준수)

```
unset NODE_OPTIONS                      # NODE_OPTIONS 오염 회피(브리프 지시)
export PATH="$HOME/.cargo/bin:$PATH"    # cargo는 PATH에 없고 ~/.cargo/bin에 있다
bunx @tauri-apps/cli@2 build --bundles app \
     --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

함정 회피 근거:
- **updater 서명키 hard-fail**: `createUpdaterArtifacts=true`(conf L25)라 서명키 없이는 빌드 말미에
  실패한다 → 일회용 키를 만들지 않고 **인라인 config로 그 항목만 off**(키 자산은 보관·폐기 부담을 낳는다).
- **DMG osascript 함정**: `.dmg` 단계가 GUI 권한 없는 에이전트 셸에서 실패한다 →
  **`--bundles app`으로 `.app`만** 생성해 원천 회피. 설치는 `.app` `ditto`로 충분하다.
- 귀결: **updater 아티팩트·dmg 부재 = 수동 설치 전용**(의도된 것이며, 설치는 박사님 실셸 몫).

### 결과

```
BUILD_START=2026-08-02T20:25:06+09:00
BUILD_END  =2026-08-02T20:26:20+09:00      (74초 — target/ 캐시 온난)
BUILD_EXIT =0
Finished 1 bundle at: target/release/bundle/macos/cys.app
```

### 산출물 실측

| 항목 | 값 |
|---|---|
| `Info.plist` CFBundleShortVersionString | **0.14.10** |
| 사이드카 `cys --version` | `cys 0.14.10` |
| 사이드카 `cysd` | `[cysd] v0.14.10 cys-fix-w2-gen-0.14.4` |
| 동봉 팩 | `Contents/Resources/pack.tar.gz` (2,338,874 B · 456파일 결정론 동봉) |
| 서명 | `Signature=adhoc` (linker-signed) — **우리가 codesign을 실행한 것이 아니다** |

### stale 산출물 오판 차단

빌드 **전** 같은 경로에 **0.13.20 / Jul 27 16:15** 산출물이 있었다. 두 축으로 새 빌드임을 확정:
- 버전: `0.13.20` → **`0.14.10`**
- mtime: `Jul 27 16:15:03` → **`Aug 2 20:26:19`**

### UI 커스텀 임베드 확인 (`strings`로는 확인 불가 — 2축 대조)

Tauri는 프런트엔드를 `generate_context!`로 **압축 임베드**하므로 바이너리에 `strings`를 걸어도
UI 마커가 0건 나온다(장기기억 함정 3). 대신 2축으로 확인:

- **축1 — `ui/dist/main.js` 안의 우리 커스텀 마커 실재**:
  `cys-title-size` 2건 · `cys-term-weight` 2건 · `cys-title-color-role` 1건 · `cys-menu-weight` 2건
- **축2 — 신선도**: `dist/main.js` `20:25:09` **<** 바이너리 `cys-app` `20:26:19`
  ⇒ 이번 빌드가 이번 dist를 먹었다.

---

## 6. 금지선 준수 (전부 미실행)

| 금지 항목 | 상태 |
|---|---|
| origin/upstream push | **미실행** — 로컬 브랜치 `rebase/v0.14.10`에만 존재 |
| `/Applications` 설치 | **미실행** — 산출물은 `target/` 안에만 있다 |
| cysd/앱 재시작 | **미실행** |
| `codesign` 재서명 | **미실행** — `adhoc`은 링커 기본값이지 우리 서명이 아니다 |

설치는 **박사님 실셸 몫**이다. 참고로 로컬 빌드본은 quarantine이 애초에 붙지 않아
`xattr` 단계가 불필요하다.

---

## 7. 남은 위험·후속 (판단은 master·박사님 몫)

1. **⚠ 설치 후 `init-pack` 방아쇠** — 정책 §5 경로 B 단서: 자체 빌드본 첫 기동에서 `init-pack`이
   여러 번 호출되며 `pack.prev`를 갈아치운다. **정책 §3 외부 수동 백업이 선행되어야** 롤백 담보가 생긴다.
   이 빌드에는 `pack.tar.gz`(456파일)가 동봉돼 있어 첫 기동 시 팩 병합이 실제로 일어난다.
2. **⚠ 정책 §6 사후 검증 7번** — 설치 후 `cys schedule list | grep -i formation` **0건**을 반드시 확인.
   있으면 즉시 `cys schedule remove formation-ensure-10min`(07-27 실사고 재발 방지).
3. **⚠ TCC 재승인** — 서명이 바뀌면 macOS TCC가 리셋되어 페인 안 Claude가 구글 드라이브 폴더
   접근에서 EPERM으로 죽을 수 있다(정책 §5). 유지보수 창 확보 권고.
4. **선재 타입오류 8건** — upstream 결함이라 우리 범위 밖이나, 우리가 이 포크를 계속 쓰는 한
   남는다. 수정 여부는 별도 판단 대상(이번 티켓에서는 **손대지 않았다** — 외과적 변경 원칙).
5. **updater 아티팩트 부재** — 이 빌드는 인앱 업데이트 경로를 갖지 않는다(의도).

---

## 8. 재현 명령 요약

```bash
cd ~/cys-terminal-src
git log --oneline v0.14.10..rebase/v0.14.10        # 13커밋
cd ui && unset NODE_OPTIONS && bun test            # 215 pass / 0 fail
bunx tsc -p tsconfig.check.json                    # 비-테스트 8건(=베이스라인 동일)
cd .. && unset NODE_OPTIONS && export PATH="$HOME/.cargo/bin:$PATH"
bunx @tauri-apps/cli@2 build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'
plutil -extract CFBundleShortVersionString raw target/release/bundle/macos/cys.app/Contents/Info.plist
```
