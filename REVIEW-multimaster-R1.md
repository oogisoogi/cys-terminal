# 멀티마스터 충돌 진단·설계 — 적대검증 패킷 (R1)

> 작성: master (claude-ysfuture, 외부 세션). 검증: agy(gemini)·codex.
> 목적: 아래 **진단 주장(C1~C6)** 과 **설계(D1~D3)** 를 적대적으로 **반박**하라.
> 규칙: 지정 파일/범위만 검토. 무관 repo·파일 배회 금지. 추측 금지 — 각 판정에 **증거 file:line** 첨부.
> 점수(0-100) 금지. verdict = ACCEPT | REVISE | BLOCK | ESCALATE 중 하나.

---

## 0. 배경 (박사님 요구)

박사님은 **복수 workspace에 각각 별도 master(부서장)** 를 세우고, 별도 ws의 **CEO(master of master)** 가
부서장들만 지휘(워커 직접 관할 안 함)하는 구조를 원한다. 증상: 새 surface를 master로 선언하면
"surface:114에 이미 master가 살아있다 … 기존 생태계를 죽이거나 중복 부팅하지 않고 상태부터 실측한다
(디렉티브 §2)" 가 떠서, 박사님은 "이대로 가면 기존 ws의 모든 작업이 kill된다"고 우려.

박사님 결정: **(A) 새 ws = 자동 격리** (＋ = 전용 데몬+부서장, 기본 데몬=CEO 전용) · **(C) CEO fleet 현행 유지**.

---

## 1. 진단 주장 (각각 반박 대상)

### C1 — "kill된다"는 코드 동작이 아니다 (거부일 뿐)
같은 데몬에 살아있는 master가 있을 때 2번째 `claim-role master`는 **거부(claim_denied)** 되고, 기존
master·생태계는 **죽지 않는다**. 박사님이 본 "죽이거나 중복부팅" 문구는 master LLM의 자연어 추론이다.
- 증거: `/Users/cys/dev/cys-terminal/src/bin/cysd/handlers.rs:1203-1227` (privileged role(master|cso)을
  live surface가 점유 중이면 `role.claim_denied`/`claim_denied` 반환, kill 없음. self 재claim·죽은 보유자
  승계만 허용).
- 라이브 반증 부재: 현재 master 3개(기본 surface282 · dept-1 surface2 · dept-3 surface1)가 **동시 생존**.

### C2 — 격리는 데몬 단위, CEO 승격은 성공함
부서 격리 단위 = (socket 부모 디렉토리, pack_dir) 쌍 = 독립 데몬. CEO 승격은 실제로 일어났다.
- 증거: `~/.cys/depts.json` (dept-1·dept-3 각자 독립 socket·pack_dir) · 라이브 데몬 53460(dept-1)·60101(dept-3)
  각자 master+생태계 · `~/.cys/pack/directives/MASTER_DIRECTIVE.md` 내용 == `CEO_TEMPLATE.md` (승격 완료).
- 함의: 부서생성·격리·승격 **메커니즘 자체는 정상**.

### C3 — 근본원인: 새 master가 "공유 기본(CEO) 데몬"에 얹힘
새 워크스페이스가 격리 데몬이 아니라 **기본 데몬**에 합류해서, 데몬-전역 명령이 기존 생태계와 충돌한다.
- 증거(GUI): `/Users/cys/dev/cys-terminal/ui/src/main.ts:3348` `btn-ws-new`→`addWorkspace()`(2170, `socket=undefined`=기본 데몬)
  vs `:3350` `btn-ws-dept`→`addDeptWorkspace()`(2193, `allocate_dept_daemon`=격리 데몬).
- 증거(부트): `~/.cys/pack/directives/MASTER_DIRECTIVE.md §0` ②`cys claim-role master`·④`cys boot`·`cys list`
  전부 **데몬-전역**(workspace 범위 한정 없음). §2 = "파괴적·비가역 행동 전 의도 확인"(박사님이 본 문구 출처).
- 정황: 박사님 예시 surface 번호 114·122 = 중간대 → 기본 데몬 누적 카운터(현재 282)와 일치, 부서 데몬(1~7)
  아님 → 새 master가 기본 데몬에 떴음.

### C4 — Fix 1이 충돌을 "구조적으로" 제거
새 ws를 빈 격리 데몬에 띄우면, 그 데몬엔 master가 없어 `claim-role master`가 충돌 없이 성공한다.
- 증거: `~/.cys/pack/bin/cys-dept` `allocate`(178-225)·`launch`(160-177)는 데몬만 spawn(빈 데몬). `create`(226-342)만
  `javis_boot_node --role master`(329-331)로 부서장 자동 각성. → 빈 데몬엔 master 부재 → C1의 claim_denied 가드
  (handlers.rs:1203-1227)가 발동 안 함.

### C5 — 곁가지 결함 3종
- C5a: `cys-dept`가 PATH에 없음 (`~/.cys/pack/bin/cys-dept`만 존재, `/opt/homebrew/bin`·PATH 부재) → CEO
  디렉티브 `cys-dept list` fan-out이 풀경로 없이 실패. 증거: `which cys-dept` 실패.
- C5b: dept-2 고아 — `~/.cys/pack-dept-dept-2` 존재하나 `depts.json` 미등록·데몬 없음(`cys-dept-dept-2/cys.sock` no socket).
- C5c: CEO 데몬이 풀 워커 부팅 — 부트 §0 ④ `cys boot`가 CEO/부서장 구분 없이 cso+worker+리뷰어2 기동.
  (박사님 결정 C=현행 유지이므로 결함 아님·정보용).

### C6 — 박사님 모델 == 구현된 Model A
박사님 구상(부서장 master 복수 + 별도 CEO)은 이미 구현된 Model A와 동일. 빠진 건 아키텍처가 아니라
**(a) 진입 배선 + (b) 부트 시퀀스의 데몬 자각** 두 가지.

---

## 2. 설계 (각각 반박 대상)

### D1 — Fix 1: 진입 배선 자동격리 (로컬 가역)
`ui/src/main.ts`의 `btn-ws-new`(3348)/`addWorkspace()`(2170)를 **격리 데몬 경로**로 전환 —
`allocate_dept_daemon`로 전용 소켓을 받아 ws를 붙임(기존 `addDeptWorkspace` 무명 분기 재사용).
기본 데몬엔 새 ws 미합류(= CEO 전용). (선택) `cys-dept allocate`에 `javis_boot_node --role master` 추가로 부서장 자동각성.
- 리스크 후보: ＋마다 cysd 데몬 1개 증가(watchdog·scheduler 동반·자원 누적) · allocate 무명 dept-N 명명 ·
  기존 default-daemon ws(첫 ws=CEO) 와의 마이그레이션 호환성 · GUI가 만든 첫 pane과 자동각성 master의 더블생성 가능성.

### D2 — Fix 2: 부트 데몬 자각 안전망 (헌법 변경=박사님 토큰 필요)
신규 결정론 게이트(`javis_preflight.py` C-check `misrouted-master`): `cys identify`+`cys list` 대조 →
내 surface 아닌 live master 존재 시 `MISROUTED_MASTER` → 부트가 `cys boot`/survey 중단·박사님 보고(kill 추론 금지).
Fix 1이 충돌을 구조 제거하므로 **순수 이중방어**(수동/명령팔레트로 기본 데몬에 master 선언하는 잔여 경로 대비).
- 리스크 후보: 자기 surface 식별(외부 세션은 caller=null) 시 오탐 가능 · idempotent re-claim과의 구분 · §12 결정론 환원과의 정합.

### D3 — Fix 3: 위생
- C5a PATH 래퍼 (로컬 가역) · C5b dept-2 tar 백업 후 정리(비가역=박사님 승인·CSO 직할) · C5c 현행 유지.

---

## 3. 회신 형식 (verdict 계약)

각 항목(C1~C6, D1~D3)에 대해:
```
[항목ID] verdict=<ACCEPT|REVISE|BLOCK|ESCALATE>
  evidence: <file:line 또는 라이브 명령 결과>
  근거/반박: <1~3줄. 반박이면 무엇이 틀렸고 무엇으로 대체해야 하는지>
```
추가로: **놓친 실패모드·리스크**(특히 D1 자동격리의 자원증식·더블master·복원 호환), **C1의 'kill 없음' 재검증
결과**(handlers.rs 직접 확인), **이 진단이 빠뜨린 다른 근본원인 후보**가 있으면 명시.
회신은 master 채팅으로 수합되도록 `cys send --to master "[리뷰어명 R1] ..."` 로 push(또는 이 surface에 출력).
