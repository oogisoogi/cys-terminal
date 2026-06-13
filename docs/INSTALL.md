# cys 터미널 설치 가이드

> cys는 **AI 조직(자비스)이 거주하는 터미널**입니다. 코어 데몬(cysd) + CLI(cys) + GUI 앱(cys.app)으로
> 구성되며, **받는 사람은 데몬을 따로 설치할 필요가 없습니다** — 앱·CLI가 자동으로 기동합니다.

## 무엇이 자동인가 (0.2.0+)

신규 머신에서 별도 설정 없이 동작합니다:

1. **데몬 자동 기동** — 앱을 열거나 `cys` 명령을 처음 쓰면, 데몬이 없을 때 번들에 동봉된 `cysd`를
   분리(detached) 프로세스로 자동 기동합니다. (옵트아웃: `CYS_NO_AUTOSTART=1`)
2. **Pack 자동 설치** — 데몬 첫 기동 시 `~/.cys/pack`(디렉티브·스킬·스케줄·ACL)을 자동 설치합니다
   (보존 모드 — 기존 파일은 덮어쓰지 않음).
3. **pane 내 PATH 주입** — 데몬이 만드는 모든 pane의 `PATH` 선두에 `cys` 바이너리 폴더가 들어가,
   pane 안의 AI가 `cys identify`를 바로 쓸 수 있습니다(심링크 수동 생성 불요).

→ 따라서 GUI만 쓰는 사용자는 **앱을 더블클릭하면 끝**입니다.

## macOS

### A. 설치파일 (DMG) — 권장
1. `cys-0.2.0-macos-arm64.dmg`를 열고 `cys.app`을 `Applications`로 드래그.
2. 첫 실행 시 Gatekeeper 경고가 나오면: 우클릭 → "열기"(미서명 빌드의 경우).
   서명·공증된 빌드라면 바로 열립니다.
3. 끝. 앱이 데몬을 자동 기동합니다.

### B. CLI도 터미널에서 쓰려면 (선택)
앱 번들 안의 바이너리를 PATH에 노출합니다:
```sh
sudo ln -sf /Applications/cys.app/Contents/MacOS/cys  /usr/local/bin/cys
sudo ln -sf /Applications/cys.app/Contents/MacOS/cysd /usr/local/bin/cysd
```
(pane *안*에서는 PATH가 자동 주입되므로 이 단계는 박사님이 **앱 밖 터미널**에서 `cys`를 칠 때만 필요)

### C. 24/365 상시 가동 (선택 — 헤드리스/무인 운영)
재부팅 후에도 데몬이 자동으로 살아 있게 launchd에 등록:
```sh
cys daemon install            # 로그인 시 자동 기동 + 사망 시 자동 재기동(KeepAlive)
cys daemon status             # 등록·가동 상태 확인
cys daemon uninstall          # 해제
```
- 이미 데몬이 떠 있으면 `cys daemon install`은 안전을 위해 거부합니다(flock 충돌 방지).
  기존 세션을 정리하고 launchd로 소유권을 넘기려면 `cys daemon install --takeover`
  (⚠ 가동 중인 세션이 소멸합니다 — `cys list`로 먼저 확인).

## Windows

> 코어(named pipe·ConPTY·DSR)는 검증 완료(docs/WINDOWS_VALIDATION.md). GUI 앱(Tauri)의 Windows
> 빌드는 별도 트랙입니다. 현재 Windows 배포는 **CLI+데몬(MSI/ZIP)** 중심입니다.

### A. MSI 설치파일
1. 아키텍처에 맞는 파일 실행: `cys-0.2.0-windows-x64.msi` 또는 `...-arm64.msi`.
2. 설치 시 `cys.exe`·`cysd.exe`가 PATH에 등록됩니다.
3. `cys list` — 데몬이 없으면 자동 기동됩니다.

### B. ZIP (수동)
`cys-0.2.0-windows-x64.zip`을 풀고 폴더를 PATH에 추가하거나, 그 폴더에서 직접 `cys.exe` 실행.

### C. 상시 가동 (선택)
```
cys daemon install     # 로그온 시 자동 기동 (작업 스케줄러 ONLOGON)
cys daemon status
cys daemon uninstall
```
※ Windows는 작업 스케줄러에 사망-시-재기동이 없어, CLI 자동기동이 이를 보완합니다.

## 빠른 시작 (설치 후 공통)

```sh
cys status                                   # 전 노드 1콜 관제 보드
cys launch-agent --role master --agent claude  # 역할 에이전트 기동 + 디렉티브 자동 주입
cys boot                                     # 설치된 CLI 자동 감지 → worker+reviewers 일괄 기동
```

`agents.json`(`~/.cys/pack/agents.json`)에서 각 에이전트의 기동 명령을 환경에 맞게 수정하세요
(claude/gemini/codex/grok 어댑터 정의).

## 제거

```sh
cys daemon uninstall                         # 상시 가동 해제(설치했다면)
# macOS: Applications에서 cys.app 삭제 + /usr/local/bin/cys{,d} 심링크 제거
# Windows: 제어판에서 MSI 제거
rm -rf ~/.cys ~/.local/state/cys             # pack·트랜스크립트·상태 완전 삭제(선택)
```

## 환경 변수 (요약)

`CYS_SOCKET` · `CYS_NO_AUTOSTART`(1=CLI 자동기동 끔) · `CYS_PACK_DIR` ·
거버넌스: `CYS_LOAD_THRESHOLD`·`CYS_PROC_THRESHOLD`·`CYS_DUP_THRESHOLD`·`CYS_AUTOKILL_DUP`·`CYS_IDLE_SECONDS` ·
자비스: `CYS_TYPING_GUARD_SECS`·`CYS_FEED_REMIND_SECS`·`CYS_MASTER_DEADMAN_SECS`·`CYS_AGENT_AUTORESTART`·`CYS_RECALL_RETAIN_DAYS`·`CYS_TODO_DIRS`
(상세는 README.md)
