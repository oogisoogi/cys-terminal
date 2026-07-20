# 설계 v3: 레이아웃 커스텀 영구화 (cys UI)

> 작성 2026-07-20 · cys-dev(node227) · 박사 07-20 확정 과제1 재설계
> **v2: codex(surface:228) BLOCK/NO-GO 9지적 전면 반영.**
> **v3(master 07-20 수정지시): codex 재호출 금지(228 폐쇄·구독소진). 재검증=master 경검토
>   (정식 hetero 재검증은 구독 리셋 Jul25 이후 이연). G3 명명템플릿=박사 '옵션'→이연.
>   phase1(G1+G2)만 승인. ★구현은 '설계 개정→master 확인' 후에만 착수.**
> 상태: **master 경검토 대기 · 빌드 미착수 · phase1 구현은 master 확인 후**
> 방침: 빌드·설치·재시작 미실행(검증만) · 전달=소스 커밋(format-patch) · 과제1+2 함께 전달

## 0. 한 줄 요약

박사가 드래그로 만든 페인 배치·비율을 **cys/데몬 재시작에도 복원**하고, 저장을 `~/.cys`
사용자 데이터로 옮긴다. 자동 4열 타일링(4bf8aa3)은 **폐기가 아니라 "저장본 없을 때의
초기 폴백"으로 강등**한다. (명명 템플릿 G3 = 박사 옵션 → phase2로 이연.)

## 0-1. v1 대비 변경 요지 (codex BLOCK 해소)

| # | codex 지적 | v2 결정 |
|---|---|---|
| 1 | `role+ord`는 identity 아님 | **폐기**. 안정키 = `(socket, role, cwd, agent)`. `ord`는 표시순서 전용 |
| 2 | 접기+고아 append로 topology 비가역 손실 | **settle-window reconciliation** — 미매칭 리프를 placeholder로 보존, 자동 셸 생성·일렬 append 금지 |
| 3 | 이중 진실원 미확정 | **`~/.cys/ui-layouts.json` 단일 authority**, localStorage=1회 마이그레이션 후 tombstone |
| 4 | atomic rename은 torn write만 방지 | **revision CAS + flock**(prod/dev 동시쓰기 대비) |
| 5 | 과도기 UX 미규정 | **staged gate** — 1b를 prod identifier로 먼저 검증 후 dev 분리 활성 |
| 6 | legacy leaf 재붕괴 | 첫 매칭 시 **즉시 안정키 승격·flush** |
| 7 | flush 순서 역전 | **직렬 write 큐 + monotonic revision** + mouseup·close·visibility flush |
| 8 | 손상파일 자동덮어쓰기 | NotFound/corrupt/permission **구분**, corrupt는 백업·비덮어쓰기·에러 UI |
| 9 | 과제2 전역 lsregister 과도 | **targeted unregister** + dev wrapper의 bundle-id assertion |

**master 07-20 요구수정 7 ↔ 위 표 매핑(전부 반영 확인)**: ①stable instance_key=행1(§3-1 A1
권고) · ②placeholder+점진 reconciliation=행2 · ③~/.cys 단일 SOT+마이그레이션/tombstone=행3 ·
④revision CAS/lock 또는 daemon 단일writer=행4(§7-D) · ⑤staged prod→dev migration=행5 ·
⑥직렬 flush·손상보존·적대테스트=행6·7·8+§6 매트릭스 · ⑦localStorage vs file authority 확정=행3(§7-E).

---

## 1. 현재 실측 (파일·라인 근거)

| 기능 | 상태 | 근거 |
|---|---|---|
| 드래그로 분할 비율 조정 UI | **이미 존재** | `ui/src/main.ts:2055` `attachDividerDrag`→`node.ratio`→`saveLayout()` |
| 페인 자체를 드래그로 재배치 | **없음**(divider는 크기만) | Split→/↓·정렬로만 위상 생성 |
| 레이아웃 저장 | **이미 존재** | `main.ts:1384` `saveLayout`→localStorage `cys-layout-v2` |
| 레이아웃 복원 | **존재하나 sid 종속** | `main.ts:4095~`·죽은 sid 제거 `:4189`·고아 일렬 append `:4227-4231` |
| **surface_id 는 데몬 재시작마다 1부터 재발급** | **코드 확증** | `src/bin/cysd/recall.rs:382` 주석("재시작마다 1부터 재발급하면 무관 세션이 같은 id로 recall에 합쳐진다") |
| 데몬 측 surface-layout 영속·복원 | **없음** | recall.rs = 트랜스크립트 **검색** DB(FTS5)일 뿐. 재기동은 `cys boot`가 재생 |
| `boot` 표준 편성 재생 | 고정 (role,agent) 세트 | `src/bin/cys.rs:3719` — CSO·worker(claude)·reviewer agy·codex 4종 의무 + grok |
| surface durable 속성 | role·cwd·agent | `src/bin/cysd/state.rs:101` Surface{role, cwd, agent_meta(name)} |
| 앱↔`~/.cys` 파일 R/W | **검증된 command 패턴** | `src-tauri/src/main.rs:976` `read_profile_audience`·`:446` `std::fs::write` |

**핵심 확증**: 데몬 재시작 시 sid는 무의미(1부터 재발급). 안정 identity의 **유일한
출처는 launch identity** — `boot`이 동일 (role, agent, cwd)로 표준 편성을 재생하므로
`(socket, role, cwd, agent)` 튜플이 재시작을 넘어 재현된다.

**단, 정직한 한계(codex 지적 1·missing)**: 이 재현은 *표준 편성*에 한하며 코드가
보장하는 계약은 아니다. **generic 중복**(같은 socket·role·cwd의 plain worker 2개)과
**role:null 일반 셸**은 안정키가 없다 → 아래 3-1에서 명시적 폴백·약결속으로 처리.

---

## 2. 진짜 Gap
- **G1(핵심)** 데몬 재시작 시 sid 재발급(recall.rs:382 확증)으로 저장 tree가 전부 dead
  판정 → `dir:"row"` 일렬로 붕괴. = 박사가 겪는 "재시작 복원 실패"의 근본.
- **G2** persist가 identifier 종속 localStorage(`~/Library/WebKit/com.cysjavis.terminal`).
  과제2가 identifier를 `.dev`로 분리하면 dev/prod 레이아웃이 갈라짐.
- **G3** 명명 템플릿 1/2/3 없음(2단계 옵션).

---

## 3. 데이터 모델

### 3-1. instance_key (codex#1·master① — `role+ord` identity 폐기)
```jsonc
{ "type": "pane",
  "instance_key": { "socket": "…/cys.sock", "role": "worker-eduscan",
                    "cwd": "/…/eduscan", "agent": "claude" },
  "ord": 0 }                 // ord = 표시순서 힌트 전용 · 매칭 identity 아님
```
- **1차 매칭**: `(socket, role, cwd, agent)` 완전일치(라벨 role은 dept/agent 접미로 사실상
  유일 — worker-eduscan·reviewer-codex).
- **동률·약결속(generic 중복·role:null)**: 1차로 유일 확정 안 되면 **settle 윈도우 내
  위치순(ord) 근사매칭**을 하되 `binding:"weak"`로 표기 → 사용자가 리바인드 가능. **절대
  자동 셸 생성·자동 재배치 안 함.**
- 근거·한계를 스키마 주석에 박제(관측순 ord는 identity 아님 — codex#1).

#### ★master 검토 결정점 A — instance_key를 어디서 얻나 (2택)
| 안 | 내용 | 장 | 단 |
|---|---|---|---|
| **A1 (권고·phase1 채택)** 파생 튜플 | UI가 `(socket,role,cwd,agent)`를 launch identity에서 파생 | **데몬 무변경**·즉시 구현·표준 4열 편성(박사 실사용)엔 유일 | generic 중복·role:null은 약결속(위치근사)로만 |
| A2 데몬 네이티브 stable_key | 데몬이 노드 생성 시 durable instance_key 발급·재기동 재부여 | 약결속 케이스도 강결속 | **데몬 코드·프로토콜 변경 = 범위 확대**·boot 재부여 계약 신설 필요 |

> **권고**: phase1은 **A1**. 박사 핵심 요구(4열 fleet 배치의 재시작 복원)는 라벨 role이
> 유일해 A1로 충족된다. A2는 약결속이 실무에서 부족할 때의 phase1.5 경화로 남긴다.
> recall.rs:382(sid 재발급 확증)로 A2 없이는 sid 복원이 불가함은 이미 규명 — A1이 그 공백을
> 데몬 무변경으로 메운다. **최종 채택은 master 경검토에서 확정.**

### 3-2. Settle-window reconciliation (codex#2·missing 반영)
복원을 **상태기계**로: 비동기로 3/5→5/5 도착해도 topology 불변.
```
LOAD    저장 topology 그대로 구성 — 모든 리프를 placeholder(대기)로 표시
BIND    surface 도착마다 안정키로 placeholder에 결속(1차→약결속 순)
SETTLE  타임아웃(예 4s·boot 편성 도착 여유)까지 위 반복. 이 동안:
        · 미결속 placeholder = 접지 않음(자리 보존·흐리게 "대기 중")
        · 미매칭 live surface = 일렬 append 안 함(보류 목록)
RESOLVE settle 후:
        · 남은 placeholder = "없음 — [시작] [제거]"(명시 조작 전까지 자리 유지)
        · 남은 orphan = "새 페인 — [여기 바인드] [무시]"
```
→ v1의 "접기+고아 일렬 append"(main.ts:4189/4227) 폐기. 지연도착이 자리를 안 뺏는다.

### 3-3. `~/.cys` persist (codex#3 단일 진실원)
```
~/.cys/ui-layouts.json    # 신규 · pack tar 경로와 무충돌(agents.json/soul 회피)
```
```jsonc
{ "schema": 1,
  "revision": 42,                         // monotonic · CAS 기준(codex#4·#7)
  "current": { /* slot 리프 topology */ },
  "templates": [ {"id":1,"name":"개발","layout":{}}, … ] }   // G3
```
- **단일 authority = 이 파일.** localStorage `cys-layout-v2`는 **1회 마이그레이션
  소스로만** 사용 → 성공 원자쓰기 후 tombstone(`{migrated:true}`로 치환+키 clear)로
  stale resurrection 차단(codex#3·#6).

---

## 4. Persist 배선 (codex#4·#7·#8)

기존 검증 패턴(`read_profile_audience`) 위에 command 2개 신설:
```rust
#[tauri::command] fn read_ui_layouts() -> Result<Value,String>          // NotFound→null · corrupt→Err(구분)
#[tauri::command] fn write_ui_layouts(data, base_revision) -> Result<u64,String>  // CAS
```
- **CAS + flock(codex#4)**: prod/dev 동시 실행 대비. `write`는 flock 취득 → 디스크
  revision이 `base_revision`과 같을 때만 rename 커밋(revision+1 반환). 다르면
  `Err("stale")` → 호출측 reload 후 재적용. atomic rename만으로는 lost update 못 막음.
- **직렬 write 큐 + monotonic revision(codex#7)**: UI는 write invoke를 직렬화(coalescing).
  완료 역전된 async가 최신 revision을 덮지 않게 revision 가드. flush 트리거 = 드래그
  mouseup·window close·`visibilitychange(hidden)`. **선행 작업: `saveLayout` 호출부 전수
  감사**(연속 쓰기 지점 식별).
- **내구 쓰기(codex#8)**: 같은 디렉터리 유니크 temp → fsync(file)+fsync(parent dir) →
  rename. 읽기는 **NotFound(→기본 생성)·corrupt JSON(→`.corrupt`로 백업·기본 로드·에러
  UI·자동덮어쓰기 금지)·permission(→사용자 표면화)** 3분기. accept 전 schema·크기 검증.

---

## 5. UI 상호작용
- **5-1 자동 4열 강등**: `actionEqualize`(정렬 버튼)는 명시적 사용자 액션으로 유지. 변경은
  **초기 폴백만** — 저장·마이그레이션 산출이 모두 없을 때만 자동 4열.
- **5-2 명명 템플릿 (G3 · phase2 이연)**: 박사 '옵션' → phase1에서 제외. 참고 방향만:
  상단바 "화면구성 ▾"로 슬롯 명명·전환, template은 항상 topology 보존(placeholder 포함).

---

## 6. 단계·게이트 (master 07-20 phasing · 전부 소스 커밋)

**★phase1(G1+G2)만 승인. 구현은 이 설계의 master 경검토 확인 후 착수(개정 전 구현 금지).**

| 단계 | 내용 | 게이트 |
|---|---|---|
| **1a** | instance_key slot(A1 파생) + settle reconciliation(순수함수·상태기계 단위테스트) | 복원 회귀 테스트 매트릭스 통과 |
| **1b** | `~/.cys/ui-layouts.json` persist(command 2 + CAS/flock + 1회 마이그레이션·tombstone) | **prod identifier에서 먼저** localStorage→파일 승격 검증 |
| — 게이트(master⑤) — | 1b 검증 완료 후에만 과제2 dev identifier 분리 활성 | |
| **1c** | 자동 4열 = 초기 폴백 강등 | (1a 포함 가능) |
| **2 (이연)** | G3 명명 템플릿 | 박사 옵션 승인 후 별도 |

**테스트 매트릭스(master⑥ 적대테스트)**: daemon restart · reordered launch · partial arrival(3/5→5/5)
· stale async write · concurrent prod/dev writer · corrupt JSON · legacy sid-only 승격 · role:null/generic 중복 약결속.

---

## 7. master 경검토 결정점 (codex 재호출 없이 master가 판정)
> codex(228) 폐쇄로 재호출 금지. 아래는 master 경검토에서 확정할 항목(정식 hetero
> 재검증은 Jul25 구독 리셋 후 이연). 확정되면 phase1 구현 착수.

- **A. instance_key 출처**(§3-1): A1 파생 튜플(권고·데몬 무변경) vs A2 데몬 네이티브
  stable_key(범위 확대). → 권고 A1, master 확정 요망.
- **B. 약결속 UX**: generic 중복/role:null 근사매칭 후 "리바인드?" 배지 vs 첫 등장부터 수동.
- **C. settle 타임아웃**: 기본 4s의 boot 편성 도착 실측 필요(짧으면 정상 노드가 orphan화).
- **D. 동시쓰기 방식**: flock+CAS(채택안·프로토콜 무변경) vs daemon 단일 writer(최견고·범위 확대).
- **E. authority 확정(master⑦)**: 본 설계는 **파일 단일 SOT + localStorage 1회 마이그레이션
  후 tombstone**으로 이미 확정. master 승인만 남음.
