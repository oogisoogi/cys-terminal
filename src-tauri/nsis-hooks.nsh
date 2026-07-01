; cys NSIS 설치 훅 — 업그레이드/재설치 시 실행 중인 데몬(cysd)·앱(cys)이 exe를 잠가
; "Error opening file for writing: ...cysd.exe" 로 덮어쓰기 실패하는 것을 막는다.
; Windows는 실행 중 exe를 덮어쓸 수 없으므로 파일 설치 전에 종료한다.

!macro NSIS_HOOK_PREINSTALL
  ; 데몬(분리 프로세스라 앱 종료로는 안 죽음)과 앱을 강제 종료 후 잠시 대기(핸들 해제).
  nsExec::Exec 'taskkill /F /T /IM cysd.exe'
  nsExec::Exec 'taskkill /F /T /IM cys.exe'
  Sleep 1000
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 제거 시에도 동일 — 실행 중이면 파일 삭제 실패.
  nsExec::Exec 'taskkill /F /T /IM cysd.exe'
  nsExec::Exec 'taskkill /F /T /IM cys.exe'
  Sleep 1000
!macroend
