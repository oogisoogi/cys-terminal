# ⚠ DEPRECATED (2026-06-27·박사님 결정) — 정본 = `DESIGN-noshutdown-pack-update.md` (worker 보강 R0·§7-①~⑦·agy R2 ACCEPT)

> 본 v2는 master가 worker 보강을 모르고 평행 작성한 **중복·열등본**(혼선원). 박사님 결정으로 폐기.
> 파일 완전 제거는 박사님 `! rm` 또는 CSO 위임. 아래 내용은 이력 참고용.

# DESIGN v2 — cys-terminal 무중단(재시작 0) 팩 업데이트 채널

> **master 재설계 (2026-06-27).** R0(`DESIGN-noshutdown-pack-update.md`·worker)가 codex 적대감사로 **BLOCK**(코드 전제 4개 불일치). 본 v2는 그 4개를 코드 실측(file:line)으로 바로잡고 codex 완화책을 반영한다.
> repo=`/Users/cys/dev/cys-terminal`·브랜치=`feat/multi-master-formalization`. **R0 결함은 §1, 재설계는 §2~6, 코드 실측 부록 §9.**

---

## 0. TL;DR
팩(디렉티브·스킬·스크립트·워크플로우)을 cys 재시작 없이 24/365 가동 중 반영한다. embed 경로(첫설치·복원 바닥)는 그대로 두고, **서명된 외부 아티팩트**(`pack.tar.gz`+`pack-manifest.json`을 **함께 서명**)를 실행 중 cys가 받아 검증→**atomic 디스크 반영**→**해시 기반 강제 재주입(pack-reinject)**→상주 캐시 리로드까지 한다. R0가 재사용한다던 `install`·`reinject --check`·`surface.list`는 실제로는 그 용도에 안 맞음을 코드로 확인(§1).

## 1. R0 결함 (codex BLOCK·코드 실측 확정)
| # | R0 전제 | 코드 실측 | 판정 |
|---|---|---|---|
| 1 | `reinject --check`를 idle ACK 게이트로 사용(새 디렉티브 각성) | cys.rs:3916-3919 — **ACK matched면 "재주입 불필요" return**. ACK=노드가 옛 디렉티브로 살아있음 → skip → **새 팩 미반영** | 치명·전제 반대 |
| 2 | `pack-manifest.json`(min_binary_version·pack_version) 버전판정 | 매니페스트가 **서명 대상 아님** → 변조 시 버전/호환성 우회 | BLOCK |
| 3 | `install` preserve-gate 재사용 | pack.rs:496(`PACK.chain(PACK_SKILLS)` embed만)·479(`env!("CARGO_PKG_VERSION")`)·538(`std::fs::write` non-atomic·best-effort) — **embed 결합·외부입력/atomic rollback 미설계** | BLOCK |
| 4 | 무중단 검증을 `surface.list` session_id 연속성으로 | handlers.rs:678-693 — **응답에 session_id 없음**(pid·role·agent_alive만) | BLOCK |

## 2. Signed Envelope (결함 2 해소)
- 배포 아티팩트 = `pack.tar.gz` + `pack-manifest.json`(`pack_version`·`min_binary_version`·각 파일 sha256·tar sha256). **manifest 전체 + tar를 minisign으로 서명** → `pack.sig`.
- cys에 **공개키 내장**(`include_str!("keys/pack-update.pub")`·embed). 검증: `pack.sig`로 manifest 서명 확인 → manifest의 tar sha256 == 실제 tar → tar 추출 후 각 파일 sha256 == manifest. **하나라도 불일치 = 거부**(악성·변조 차단).
- `min_binary_version` ≤ 현재 바이너리 버전 아니면 거부(호환성). 라이브러리=`minisign-verify`(순수 Rust·검증 전용·경량).

## 3. install_from_iter + atomic (결함 3 해소)
- 현 `install(force)`(pack.rs:467-611)을 **입력원 추상화**로 리팩터:
  `install_from_iter(files: impl Iterator<Item=(rel, content, exec_bit)>, pack_version: &str, force: bool)`.
  - embed 경로: `install_from_iter(PACK.chain(PACK_SKILLS), CARGO_PKG_VERSION, force)` (기존 동작 100% 보존·회귀 0).
  - 외부 경로: `install_from_iter(extracted_tar_files, manifest.pack_version, false)`.
- **재사용 로직(입력원 무관)**: preserve-gate(*_DIRECTIVE.md·사용자 수정본 보존 502-531)·prune(543-581)·다운그레이드 차단(476-492·외부 pack_version 기준)·manifest·exec-bit(593-608).
- **atomic 교체**(현 538 `fs::write` non-atomic 대체): 파일별 `temp write → fsync → rename(temp, path)`. 전체 실패 시 이미 rename된 것 **rollback**(이전 내용 temp 보관). `.pack-version`은 **전 파일 성공 후 마지막**에 범프(부분반영 가시화 차단).

## 4. pack-reinject 신규 모드 (결함 1 해소)
- `reinject --check`(ACK skip)는 새 팩 반영에 **쓸 수 없다**(§1-1). 신규 모드 `pack-reinject`:
  - 노드별 **반영 해시 추적**: 마지막 주입한 디렉티브 sha256을 데몬 surface 상태에 기록.
  - 디스크 새 디렉티브 sha256 ≠ 노드 반영 해시 → **강제 재주입**(ACK 무관·forced). 같으면 skip(멱등).
  - **busy 보호**(agy 결함): 노드 busy(작업 중)면 **queued**(idle 전환 시 데몬이 자동 배달·`--queued` 메커니즘 재사용)로 재주입 → **대화 컨텍스트 파괴 0**. idle이면 즉시.
  - reinject 후 노드 반영 해시 갱신.

## 5. 검증 shape 보정 (결함 4 해소)
- session_id 없음 → 무중단 검증을 다음 **불변식**으로:
  - **pid 집합 동등**: `daemon_pid`·cys-app pid·각 surface `pid`(handlers.rs:686 존재) 전/후 스냅샷 동일. pid 변경 = 재시작 발생 = **hard fail**.
  - **.pack-version 범프**: 전(구버전) → 후(신 pack_version) 확인.
  - **노드 반영 해시**: 대상 노드 반영 해시 == 새 디렉티브 해시(§4).
- 세 불변식 전부 충족 = 무중단 반영 성공. session_id 의존 제거.

## 6. 상주 캐시 리로드 (agy 결함 5)
- 디스크 반영(§3) 후 상주 프로세스 갱신:
  - **워커·master 노드**: §4 pack-reinject(디렉티브).
  - **MCP 서버**: 팩이 MCP 정의 변경 시 해당 MCP 재로드 신호(노드에 `/mcp reload` 또는 재기동 지시·queued).
  - **Tauri UI**: 스킬 카탈로그·디렉티브 표시 캐시 무효화 이벤트(`daemon-event` 발행 → UI가 재조회). UI 자체 재시작 없음.

## 7. 라우팅 (R0 §3 계승)
- embed install(첫설치·복원·오프라인 바닥)·Tauri updater(바이너리 변경) 경로는 **그대로**. 본 채널은 **팩만의 변경**을 무중단 반영하는 추가 경로. 충돌 없음.

## 8. 불가침·리스크·박사님 결정
- **불가침 4**: ①cysd·cys-app·세션 pid 불변(재시작 0) ②서명 검증 통과 못한 팩 반영 0 ③*_DIRECTIVE.md·사용자 수정본 보존(preserve-gate) ④부분반영 가시화 0(atomic+버전 마지막).
- **리스크→완화**: 서명키 유출→키 회전·만료(manifest min_binary_version) / 추출 중 크래시→atomic rollback / busy 노드 강제주입→queued(§4) / MCP 리로드 실패→노드 재기동 폴백·로그.
- **박사님 결정 4**: ①서명키 생성·보관(minisign keypair·공개키 embed·비밀키 박사님 보관) ②외부 아티팩트 배포 채널(어디서 다운로드·gh release? 자체?) ③min_binary_version 정책 ④자동 적용 vs 수동 트리거(무중단이라도 적용 시점).

## 9. 코드 실측 부록 (file:line)
- reinject: `cys.rs:3889-3937`(run_reinject), check ACK skip=3916-3919, compose_directive 재주입=3922-3923.
- install: `pack.rs:467-611`, embed iter=496, CARGO_PKG_VERSION=479, non-atomic write=538, preserve-gate=502-531, prune=543-581, 다운그레이드=476-492, manifest/version write=585-589.
- surface.list: `handlers.rs:678-697`(session_id 부재·pid=686).
- pack embed: `pack.rs:10-358`(PACK), include skills=362.
- Tauri 재시작: `src-tauri/main.rs:1126`(app.restart), pending update=489-494.

> **R1 적대검증 대상**: 본 v2가 R0 결함 4를 닫았는지 + 신규 결함(서명키 운영·atomic rollback 완전성·queued 재주입 경합·MCP 리로드 신뢰성)을 agy·codex가 검증.
