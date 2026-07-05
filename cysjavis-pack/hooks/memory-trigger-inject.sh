#!/bin/sh
# W2-4 UserPromptSubmit hook 래퍼 — 라이브 배선 승인 시 ~/.cys/pack/hooks/ 로 복사 등록.
PACK="${CYS_PACK_DIR:-$HOME/.cys/pack}"
exec python3 "$PACK/bin/javis_memory_inject.py"
