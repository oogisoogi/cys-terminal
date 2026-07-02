; cys NSIS 설치 훅 — 업그레이드/재설치 시 잠긴 exe("Error opening file for writing") 문제를
; ★무중단(rename-swap · 2026-07-02)으로 푼다. 종전 taskkill cysd는 마스터·워커·부서 PTY가
; 전부 cysd 소유라 "업데이트 = 전 세션 사망"이었다(사용자 불안의 근원).
;
; 원리: Windows는 실행 중 exe의 '덮어쓰기'는 금지하나 '이름 변경(rename)'은 허용(NTFS 동일볼륨).
;   ① GUI(cys-app.exe)만 종료 — 얇은 클라이언트(main.rs:1 "UI가 죽어도 세션(PTY)은 데몬에
;      살아있다. UI 재시작 = 재attach") → 세션 무손실.
;   ② cysd.exe·cys.exe는 죽이지 않고 옆(.prev*.exe)으로 rename → 새 exe를 제자리에 설치.
;      구 데몬은 renamed 파일로 계속 봉사(lame-duck)하고, 새 cysd 기동이 잔해를 청소한다.
;   ③ rename이 전부 실패할 때만 구 방식(taskkill) 폴백 — 현재보다 나빠질 수 없는 우아한 강등
;      (그 경로는 기존 재시작 복원(maybe_apply_pending_update → cys restore)이 받친다).
; prev 체인(prev→prev2→prev3): 직전 lame-duck이 아직 살아 prev 파일을 점유 중일 수 있어
; 고정 3칸으로 우회한다(연속 lame-duck 2개 초과는 실사용상 희귀 — 초과 시 폴백 kill).

!macro NSIS_HOOK_PREINSTALL
  ; ① GUI만 종료(세션은 데몬 소유 — 무손실). updater 경로면 이미 종료 중이라 멱등.
  nsExec::Exec 'taskkill /F /T /IM cys-app.exe'

  ; ② cysd.exe rename-swap (데몬 무사망)
  IfFileExists "$INSTDIR\cysd.exe" 0 cysd_done
  Delete "$INSTDIR\cysd.prev.exe"          ; 잔해 청소 시도(구 lame-duck 점유 시 실패해도 무시)
  ClearErrors
  Rename "$INSTDIR\cysd.exe" "$INSTDIR\cysd.prev.exe"
  IfErrors 0 cysd_done
  Delete "$INSTDIR\cysd.prev2.exe"
  ClearErrors
  Rename "$INSTDIR\cysd.exe" "$INSTDIR\cysd.prev2.exe"
  IfErrors 0 cysd_done
  Delete "$INSTDIR\cysd.prev3.exe"
  ClearErrors
  Rename "$INSTDIR\cysd.exe" "$INSTDIR\cysd.prev3.exe"
  IfErrors 0 cysd_done
  nsExec::Exec 'taskkill /F /T /IM cysd.exe'   ; ③ 최후 폴백 — 기존 재시작 복원 경로가 받침
cysd_done:

  ; ② cys.exe rename-swap (CLI는 단명 프로세스지만 실행 중 잠금 대비 동일 처리)
  IfFileExists "$INSTDIR\cys.exe" 0 cys_done
  Delete "$INSTDIR\cys.prev.exe"
  ClearErrors
  Rename "$INSTDIR\cys.exe" "$INSTDIR\cys.prev.exe"
  IfErrors 0 cys_done
  Delete "$INSTDIR\cys.prev2.exe"
  ClearErrors
  Rename "$INSTDIR\cys.exe" "$INSTDIR\cys.prev2.exe"
  IfErrors 0 cys_done
  Delete "$INSTDIR\cys.prev3.exe"
  ClearErrors
  Rename "$INSTDIR\cys.exe" "$INSTDIR\cys.prev3.exe"
  IfErrors 0 cys_done
  nsExec::Exec 'taskkill /F /T /IM cys.exe'
cys_done:
  ClearErrors                                 ; 훅 종료 시 에러 플래그 잔류로 설치기 오판 방지
  Sleep 500
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 제거(uninstall)는 의도적 전면 종료 — 세션 보존 대상이 아니다. lame-duck(.prev*)까지 정리.
  nsExec::Exec 'taskkill /F /T /IM cys-app.exe'
  nsExec::Exec 'taskkill /F /T /IM cysd.exe'
  nsExec::Exec 'taskkill /F /T /IM cys.exe'
  nsExec::Exec 'taskkill /F /T /IM cysd.prev.exe'
  nsExec::Exec 'taskkill /F /T /IM cysd.prev2.exe'
  nsExec::Exec 'taskkill /F /T /IM cysd.prev3.exe'
  nsExec::Exec 'taskkill /F /T /IM cys.prev.exe'
  nsExec::Exec 'taskkill /F /T /IM cys.prev2.exe'
  nsExec::Exec 'taskkill /F /T /IM cys.prev3.exe'
  Sleep 1000
  Delete "$INSTDIR\cysd.prev.exe"
  Delete "$INSTDIR\cysd.prev2.exe"
  Delete "$INSTDIR\cysd.prev3.exe"
  Delete "$INSTDIR\cys.prev.exe"
  Delete "$INSTDIR\cys.prev2.exe"
  Delete "$INSTDIR\cys.prev3.exe"
  ClearErrors
!macroend
