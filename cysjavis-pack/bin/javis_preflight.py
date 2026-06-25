#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_preflight — CYSJavis 부트 결정론 프리플라이트 (절대지침의 기계 검증부).

마스터 부트 시퀀스 ⓪단계에서 반드시 실행된다. 이 스크립트가 수행하는
존재 검증·번호/역할 매핑·범위 검사·hook 등록 검사는 LLM이 자연어로 재추론하지
않는다 — 이 출력만이 유일한 사실이다 (할루시네이션 구조 차단 = 결정론 환원).

사용:
    python3 javis_preflight.py [--fix] [--json] [--skip <ID> ...]

종료 코드: 0 = FAIL 없음(WARN 허용), 1 = FAIL 존재.
의존성: 파이썬 표준 라이브러리만 (네트워크·LLM 호출 없음).
"""

import argparse
import json
import os
import re
import shlex
import shutil
import stat
import subprocess
import sys
import time

PASS, FAIL, WARN, FIXED, SKIP = "PASS", "FAIL", "WARN", "FIXED", "SKIP"

DIRECTIVES = [
    "MASTER_DIRECTIVE.md",
    "WORKER_DIRECTIVE.md",
    "CSO_DIRECTIVE.md",
    "REVIEWER_DIRECTIVE.md",
]

# 절대지침 핵심 조항의 내용 핀 — 디렉티브가 약화·소실되면 결정론으로 검출된다.
CONTENT_PINS = {
    "MASTER_DIRECTIVE.md": [
        ("오너 호칭", "호칭 규정(절대지침 G1) — 구체 호칭은 오너 주권이라 정책 라인만 핀"),
        ("javis_preflight", "부트 ⓪ 결정론 프리플라이트 편입"),
        ("60%", "컨텍스트 60% 임계 명문화(절대지침 9)"),
        ("MASTER_TODO.md", "master 자신의 todo 영속(절대지침 7)"),
        ("결정론", "결정론 환원 원칙 명문화"),
        ("세계 최고", "최고 전문가 기반 평가기준(절대지침 2)"),
        ("워크플로우 폴더", "탭 명명·작업 폴더 규칙(앵커1-b)"),
        ("지시한 내용과 근거", "워커 지시 후 오너 보고 일반 의무(앵커1-f)"),
        ("보고 채널은 master의 채팅 출력", "오너 보고 채널 명시(앵커1-f) — 의미 변형 탐지 보강 핀"),
        ("--queued", "자동 Return 배달·타이핑 가드 인지(앵커1-c)"),
        ("CYS_TYPING_GUARD_SECS", "타이핑 가드 초수 명시(앵커1-c)"),
        ("양방향 소켓통신", "양방향 소켓 절대규칙(앵커3-A)"),
        ("5분 주기 진행% 보고", "5분 주기 주인님 보고(앵커3-A6)"),
        ("javis_report.py", "진행% 결정론 산출기 편입(앵커3-A6)"),
        ("주기적 능동 점검", "능동 모니터링 강제(앵커3-B1)"),
        ("라운드 사이클 의무 단계", "라운드마다 master 주기 점검(앵커3-B5)"),
        ("CYS_IDLE_SECONDS", "idle 5분 임계 명시(앵커3-B3)"),
        ("javis_route.py", "3단 사고 라우팅 결정론 엔진 편입(사고 모드 §1)"),
        ("기억 증류", "slow 종료 게이트 증류 의무(§10)"),
        ("javis_memory.py", "증류 결정론 도구 편입(§10)"),
        ("4종 의무 노드", "LLM orchestrating 4노드 부트 의무(앵커4-1)"),
        ("javis_orchestra.py", "LLM 오케스트레이션 결정론 도구 편입(앵커4)"),
        ("5-1", "라운드 루프 5-1~5-8 명문화(앵커4-5)"),
        ("맥킨지급", "라운드 완료 기준(앵커4 5-6·5-8)"),
        ("직전 점수 +10%", "라운드 +10% 목표(앵커4 5-7)"),
        ("deep research", "gemini deep research 담당(앵커4-6)"),
        ("ChatGPT Image 2.0", "image 생성 도구 명시(앵커4-6)"),
        ("task-prompt", "위임 티켓 결정론 생성기 의무(앵커5-1·4 — 생존 게이트+4규칙 주입)"),
        ("수기 티켓 위임은 금지", "§2 위임 티켓 의무 블록 고유 핀 — 교차참조 겹침 무력화 방지(앵커5-1)"),
        ('"run command"·"update" 요청은 모두 승인', "run command·update 전부 승인(앵커5-3)"),
        ("가장 좋은 옵션", "bash 승인 즉각 최선 옵션 확인 후 승인(앵커5-2)"),
        ("무지성 승인이 아니라", "최선 옵션 '확인 후' 승인 집행문 핀(앵커5-2 — 제목만 잔존 방지)"),
        ("절대 강조 4규칙", "위임 시마다 4규칙 절대 강조(앵커5-4)"),
        ("a) **품질 절대우선**", "4규칙 a 불릿 고유 핀(앵커5-4a — §6 제목·§2 열거와 겹침 방지)"),
        ("hallucination-guard", "환각방지 전담 sub-skill 사용·생성 지시(앵커5-4b)"),
        ("몽상", "몽상·망상 촉진 절대 금지(앵커5-4b)"),
        ("Garbage-in", "토대 오염 차단(앵커5-4b)"),
        ("grill-me", "의도 합의 — 합의까지 질문 반복(앵커5-4c)"),
        ("길이는 원문 수준", "요약·압축 절대 금지·길이 보존(앵커5-4d)"),
        ("충돌 시 상위 기준 절대 우선", "②검증 동요 시 ①③ 중단·오너 보고 게이트(앵커5-4)"),
        ("자율주행 위임권", "§14 자율주행 3축 명문(앵커6)"),
        ("gate-status", "축1 게이트 4자 수렴 결정론 판정 도구 편입(앵커6)"),
        ("GATE CONVERGED", "축1 수렴 시에만 자동 전환(앵커6 — 눈대중 차단)"),
        ("축2 — 자율 컨텍스트 수명주기", "축2 불릿 고유 핀(앵커6 — 소실 검출)"),
        ("cys schedule add", "축3 자기 웨이크업 고유 핀(앵커6 — next-action 다중출현 보완)"),
        ("정지 경계 (denylist", "denylist 5종에서만 정지(앵커6 — 고유 불릿 핀)"),
        ("로컬 커밋은 가역", "외부 발행≠로컬 커밋 구분(앵커6 denylist ③)"),
        ("next-action", "축3 다음 액션 큐 결정론 추출 도구 편입(앵커6)"),
        ("kill-switch 최우선", "오너 입력=즉시 일시정지(앵커6 메타)"),
        ("Phase 종료 시 오너에게 1줄 push", "Phase 보고 의무(앵커6 메타·감사)"),
        ("품질 게이트를 무르게 하지 않는다", "자율화=전환 주체만·게이트 불변(앵커6 메타)"),
    ],
    "WORKER_DIRECTIVE.md": [
        ("_TODO.md", "워커 todo 영속(절대지침 7)"),
        ("60%", "컨텍스트 60% 임계 명문화(절대지침 9)"),
        ("set-status", "컨텍스트 자기보고 의무"),
        ("--queued", "자동 Return 배달 인지(앵커1-c)"),
        ("javis_memory.py", "slow 종료 게이트 증류 도구(§10)"),
        ("절대 강조 4규칙", "4규칙 기본 계약 — 티켓 누락 시에도 준수(앵커5-4)"),
        # 아래 핀들은 orchestra RULE_MARKERS와 패리티를 이룬다(orchestra --self-test (e)가
        # 기계 검증) — 마커 소실로 task-prompt가 폴백 강등될 때 C03이 같은 원인을 가리킨다.
        ("a) **품질 절대우선**", "4규칙 a 불릿 고유 핀(앵커5-4a) — 추출 원천 약화 전파 차단"),
        ("할루시네이션 방지", "4규칙 b 핀(앵커5-4b) — 마커 패리티"),
        ("hallucination-guard", "환각방지 전담 sub-skill 사용(앵커5-4b)"),
        ("몽상", "몽상·망상 촉진 절대 금지(앵커5-4b) — 추출 원천 핀"),
        ("Garbage-in", "토대 오염 차단(앵커5-4b) — 추출 원천 핀"),
        ("grill-me", "의도 합의 스킬(앵커5-4c)"),
        ("합의에 이를 때까지", "의도 합의 핵심 술어 — 합의까지 질문 반복(앵커5-4c)"),
        ("요약·압축 절대 금지", "4규칙 d 핀(앵커5-4d) — 마커 패리티"),
        ("전문용어·약호", "일반인 첨삭 — 전문용어·약호만 쉬운 말로(앵커5-4d)"),
        ("길이는 원문 수준", "요약·압축 금지·길이 보존(앵커5-4d) — 추출 원천 핀"),
        ("충돌 시 상위 기준 절대 우선", "②검증 동요 시 ①③ 중단·보고 게이트(앵커5-4)"),
    ],
    "CSO_DIRECTIVE.md": [
        ("CSO_TODO.md", "CSO todo 영속(절대지침 7)"),
        ("context.threshold", "60% 임계 이벤트 대응(절대지침 9)"),
        ("hallucination-guard", "환각방지 전담 sub-skill(앵커5-4b — master·CSO·워커 공통)"),
        ("몽상", "몽상·망상 촉진 절대 금지(앵커5-4b — CSO 공통 의무)"),
        ("검증 엄밀성", "3요소(검증 엄밀성·평가 신뢰성·환각 안전장치) 핀(앵커5-4b)"),
        ("master 컨텍스트 사이클 1차 집행", "축2 — CSO가 master cycle verifier(앵커6)"),
    ],
    "REVIEWER_DIRECTIVE.md": [
        ("_TODO.md", "리뷰어 todo 영속(절대지침 7)"),
    ],
}

ROLES = ["master", "worker", "cso", "reviewer"]

# Harness Creator 툴체인(오너 제작) 핀 — 2026-06-12 통합 시점 커밋.
# 스킬(pack/skills/harness-creator)은 임베드 배포되지만 이미터·검증기·게놈 툴체인은
# 6MB+ 개발 저장소라 클론 설치한다. 해석 순서: $CYS_HARNESS_HOME → ~/.cys/harness-creator
# → ~/Desktop/CYSjavis/cys-harness-creator(로컬 원본).
HARNESS_REPO = "https://github.com/idoforgod/cys-harness-creator"
HARNESS_PIN = "98a36f4b9aee761f208aa559c2e1f7c755f7c9a6"
HARNESS_KEY_FILES = ("emit_orchestrator.py", "validate_harness.py", "warrant.py",
                     "genome/soul.md")

# NotebookLM SOT 도구(nlm) 핀 — 2026-06-12 감사 커밋(v0.7.3).
# PyPI에 0.7.3+가 배포되면 "notebooklm-mcp-cli>=0.7.3" 핀으로 전환하라.
# (PyPI 0.7.2 이하는 질의 짧은답 누락·auth 오판·silent failure 미수정 — 핀 하향 금지)
NLM_MIN_VERSION = (0, 7, 3)
NLM_PIN = ("notebooklm-mcp-cli @ git+https://github.com/jacob-bd/notebooklm-mcp-cli"
           "@6d41c75e21dae89d7bf6f43a71e3095239a28281")

TODO_FILES = ["MASTER_TODO.md", "CSO_TODO.md", "WORKER_TODO.md", "REVIEWER_TODO.md"]

# 한국 법령 전용 MCP(korean-law-mcp) 핀 — 2026-06-12 감사(v4.4.1, npm) · 오너 채택.
# k-skill의 korean-law-search(프록시 경유)를 대체하는 전용 경로 — 인용 검증·판례 생사
# 확인(citator)·행위시법 판단 등 환각 방지 기능 내장. 키는 법제처 무료 OC(사람 단계).
KLAW_MIN_VERSION = (4, 4, 1)
KLAW_PIN = "korean-law-mcp@4.4.1"

# cys-video-creator 영상 자동제작 스킬(오너 제작 32종) — pack 임베드로 배포되고, C26이
# 네이티브 Claude Code(/goal) 발견을 위해 프로필 skills/ 로 심링크한다. 대표 7기둥 +
# 하위 + 공통 규약. 새 스킬 추가 시 이 목록과 pack.rs 임베드 불변식을 함께 갱신한다.
VIDEO_SKILLS = [
    "youtube-video-pipeline", "suite-runtime-keys", "cost-preview-confirm",
    "script-writer", "script-writer-research", "script-writer-structure",
    "script-writer-factcheck", "script-writer-voice-prep",
    "voice-clone-elevenlabs", "voice-clone-elevenlabs-chunk", "voice-clone-elevenlabs-synth-qc",
    "heygen-avatar-render", "heygen-avatar-render-api", "heygen-avatar-render-gate",
    "media-gen", "media-gen-image", "media-gen-edit", "media-gen-video",
    "media-gen-upscale", "media-gen-thumbnail",
    "video-stitch", "video-stitch-compositing", "video-stitch-broll", "video-stitch-captions",
    "audio-post", "audio-post-music", "audio-post-mix",
    "video-verify", "video-verify-visual", "video-verify-timing",
    "video-verify-audio-sync", "video-verify-final-gate",
]
# 영상 파이프라인이 채택하는 공식 벤더 스킬 — `npx skills add`는 cwd의 .agents/skills/에
# 프로젝트-로컬 설치한다(글로벌 아님). 그래서 preflight가 자동 실행하지 않고(엉뚱한 cwd
# 오염 방지) 영상 작업 폴더에서 사람이 1회 실행하는 단계로 안내한다(드리프트 방지·정직성).
VIDEO_VENDOR_COMMANDS = [
    "npx skills add heygen-com/hyperframes   # HyperFrames 모션그래픽 15종",
    "npx skills add elevenlabs/skills        # ElevenLabs 음성",
    "gh skill install heygen-com/skills heygen-video   # HeyGen(선택)",
]
VIDEO_RUNTIME_KEYS = ["ELEVENLABS_API_KEY", "HEYGEN_API_KEY", "FAL_KEY"]

# appbuild 웹/앱 빌드 스킬(오너 제작 20종·워커 필수) — 스펙 기반 기획→감독관 검증→자율빌드.
# pack 임베드 배포 + C27이 프로필 심링크 + 코드선행 금지 hook(PreToolUse) 등록.
# 새 스킬 추가 시 이 목록·pack.rs 임베드 불변식을 함께 갱신한다.
APPBUILD_SKILLS = [
    "appbuild", "appbuild-plan", "appbuild-plan-interview",
    "appbuild-plan-debate", "appbuild-plan-quick",
    "appbuild-screen-spec", "appbuild-screen-spec-flow", "appbuild-screen-spec-detail",
    "appbuild-tasks", "appbuild-tasks-slice", "appbuild-tasks-order",
    "appbuild-supervisor", "appbuild-supervisor-collect", "appbuild-supervisor-verify",
    "appbuild-supervisor-fix", "appbuild-supervisor-gate",
    "appbuild-orchestrate", "appbuild-orchestrate-delegate",
    "appbuild-orchestrate-verify", "appbuild-orchestrate-route",
]
APPBUILD_HOOK = "appbuild-gate.sh"  # PreToolUse 코드선행 금지 게이트
# 역할-능력 GATE hook(2번째 GATE 사례 — appbuild-gate와 동급): reviewer/planner surface의
# 변형 도구를 PreToolUse에서 deny(producer≠evaluator). matcher에 MultiEdit·Bash 포함.
CAPGATE_HOOK = "role-capability-gate.sh"
CAPGATE_HOOK_MATCHER = "Edit|Write|NotebookEdit|MultiEdit|Bash"

# C28 자기교정·영속성 hook(외부 메모리 아키텍처 접목 이관) — (스크립트, [(event, matcher)…]).
# inject/save 는 .config 구체계에서 패키지로 이관, reflect-scan·commit-nudge 는 신규.
SELFCORR_HOOKS = [
    ("inject-context.sh", [("SessionStart", None)]),
    ("save-state.sh", [("Stop", None), ("PreCompact", None)]),
    ("reflect-scan.sh", [("Stop", None), ("SessionEnd", None)]),
    ("commit-memory-nudge.sh", [("PostToolUse", "Bash")]),
]

# work management 앵커(절대지침 5차) 4규칙 b·c의 전담 sub-skill — C22가 존재·본문을 검증한다.
WORK_SKILLS = ["hallucination-guard", "grill-me"]

# 하네스 엔지니어링 운영 스킬 — C29가 3프로필에 자동 심링크(VIDEO/APPBUILD와 동일 규약).
HARNESS_SKILLS = ["harness-engineering"]
# 스킬 본문 핀 — frontmatter만 남기고 본문이 비워지면(전담 기능 소실) 결정론 검출한다.
WORK_SKILL_PINS = {
    # "원출처까지 간다"는 본문 고유 문구 — "출처 진실성"은 frontmatter description과
    # 겹쳐 순서 1 단독 삭제를 못 잡는다(적대 검증 R3).
    "hallucination-guard": ["원출처까지 간다", "근거 적합성", "논리 오류 분석", "팩트체크 판정"],
    "grill-me": ["가정 명시", "분기 질문", "모서리 사냥", "합의 선언"],
}

# 외부 에이전트 운영체계(거버넌스 점유형 스킬 모음)의 결정론 감지 시그니처 — C23.
# 충돌 정의: cysjavis가 배선된 프로필(우리 SessionStart hook 등록)과 **같은 프로필**에
# 동거할 때만 충돌이다 — 전용 프로필 분리 설치는 격리 수칙 준수로 보고 경고하지 않는다.
# (2026-06-12 gstack 감사·오너 승인: 금지가 아니라 'WARN + 격리 수칙 안내'가 목적.)
FOREIGN_AGENT_OS = {
    "gstack": {
        "skills_dir": "gstack",
        "claude_md_markers": ("skills/gstack", "/gstack-upgrade", "/land-and-deploy",
                              "open-gstack-browser"),
        "hook_marker": "gstack",
        "guide": ("격리 수칙: ①cysjavis 프로필이 아닌 전용 CLAUDE_CONFIG_DIR로 이동 "
                  "②CLAUDE.md의 gstack 섹션 제거 ③/ship·/land-and-deploy·"
                  "/gstack-upgrade 사용 금지(커밋 핀 수동 갱신만) ④hook 미등록(클론만)"),
    },
}

# 핀은 '오너 호칭' 규정 라인의 존재다 — 구체 호칭("주인님" 기본값)은 오너가 자유로이
# 바꿀 수 있어야 하므로 특정 단어를 핀으로 삼지 않는다(오너 주권과 결정론의 양립).
SOUL_MARKER = "오너 호칭"
SOUL_PLACEHOLDER = "(이름/호칭을 적어라)"
SOUL_APPEND = (
    "\n## 호칭 (절대지침 — preflight 자동 보강)\n\n"
    '- **오너 호칭: master는 오너를 "주인님"으로 호칭한다** (오너가 다른 호칭을 원하면 이 줄을 수정하라)\n'
)

# 자율주행 위임권(앵커6) — soul이 권한을 부여해야 MASTER §14가 발효된다(이 절이 없으면
# master는 자율주행하지 않는다). 오너가 회수·축소하려면 soul의 이 절을 수정·삭제한다.
# ★자동 재주입 금지(적대 검증 6차 H-2): 아래 골격은 --fix가 쓰지 않는다 — 오너가 권한을
# 다시 부여할 때 수동 복사하는 표준 문안일 뿐이다(부여 주체는 오너뿐).
SOUL_AUTOPILOT_MARKER = "자율주행 위임권"
SOUL_AUTOPILOT_TEMPLATE = """
## 자율주행 위임권 (Autonomous Pilot Mandate — 오너가 master에 부여)

- master는 승인된 로드맵을 오너 수동개입 없이 **자율 완주**할 권한을 가진다
  (MASTER_DIRECTIVE §14 — 3축: 진행권·컨텍스트 수명주기·재기동 루프).
- **정지 경계는 위 금지선(denylist)뿐이다**: 로드맵 이탈 새 범위·soul/CLAUDE/디렉티브 변경·
  외부 발행/발송·비가역 삭제·오너 명시 보유 결정권 — 여기서만 멈춰 오너 승인을 받는다.
  로컬 커밋은 가역이므로 허용된다.
- **kill-switch**: 오너의 어떤 입력이든 자율주행을 즉시 일시정지시킨다 — 오너가 항상 우선이다.
- (오너가 이 권한을 회수·축소하려면 이 절을 수정하라 — 이 절이 없으면 master는 자율주행하지 않는다.)
"""

# 자율주행 메모리 상주(앵커6 — 🔒색인 상주 필수) — C25가 파일 존재+본문 핀+색인 등재를
# 검증한다. 본문 핀: 권한·경계 실질이 비워지면(frontmatter만 잔존) 검출(WORK_SKILL_PINS 선례).
AUTOPILOT_MEMORY_FILE = "feedback_autonomous-pilot-mandate.md"
# 핀은 본문 고유 문구만 — "denylist"·"kill-switch"는 frontmatter description과 겹쳐
# 본문 문장 단독 삭제를 못 잡는다(6차 R2 N-4 — 스킬 핀 R3 교훈과 동일 계열).
AUTOPILOT_MEMORY_PINS = ["축1", "축2", "축3", "로드맵 이탈", "오너 아무 입력=즉시 일시정지",
                         "How to apply"]
AUTOPILOT_MEMORY_INDEX_LINE = (
    "- [자율주행 위임권](feedback_autonomous-pilot-mandate.md) — 3축 완전 자율주행·"
    "denylist에서만 정지·kill-switch 최우선 (🔒상주 필수 — 제거 금지)"
)

# ── C41 참조 무결성 — soul.md + directive 4종의 백틱/마커 안 named 참조가 실파일로
# 해소되는지 결정론 대조. 스캔 표면은 pack 내부 문서 그래프로 한정한다(project 메모리는
# 노드별 가변이라 비결정론 → C18 javis_memory verify 담당, 중복 회피).
REF_FEEDBACK = re.compile(r"feedback_[a-z0-9_]+")          # (a) 메모리 슬러그
REF_BIN = re.compile(r"javis_[a-z0-9_]+\.py")              # (b) bin 도구
REF_DIRECTIVE = re.compile(r"[A-Z][A-Z0-9_]*_DIRECTIVE\.md")  # (c) directive 파일명
BACKTICK_RE = re.compile(r"`([^`]+)`")
MARKER_LINE_RE = re.compile(r"^.*(상세 |🔒색인 상주|🔒상주).*$", re.M)


def _canon_feedback(s):
    """슬러그·파일명을 단일 비교 정규형으로. feedback_ 접두·.md 제거·hyphen→underscore."""
    if s.startswith("feedback_"):
        s = s[len("feedback_"):]
    return s.replace("-", "_").removesuffix(".md").removesuffix("_md").lower()


def _scan_reference_integrity(pd):
    """pack dir의 soul.md+directive 4종 백틱/마커 named 참조를 실파일과 대조.
    반환: (missing_list, scanned_count). 순수 함수 — is_dept_pack 정책과 무관(self-test가
    이 함수를 직접 호출해 FAIL/PASS를 결정론으로 증명한다 = producer≠evaluator)."""
    roots = [os.path.join(pd, "soul.md")] + \
        [os.path.join(pd, "directives", f) for f in DIRECTIVES]
    mem_dir = os.path.join(pd, "memory")
    have_feedback = {_canon_feedback(n) for n in os.listdir(mem_dir)
                     if n.startswith("feedback_")} if os.path.isdir(mem_dir) else set()
    bin_dir = os.path.join(pd, "bin")
    have_bin = set(os.listdir(bin_dir)) if os.path.isdir(bin_dir) else set()
    dir_dir = os.path.join(pd, "directives")
    have_directive = set(os.listdir(dir_dir)) if os.path.isdir(dir_dir) else set()
    missing, scanned = [], 0
    for path in roots:
        try:
            text = open(path, encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        # 마커 한정: 백틱 안 토큰 + '상세 '/'🔒상주' 마커 라인만(산문 오탐 차단).
        candidates = " ".join(BACKTICK_RE.findall(text)) + "\n" + \
            "\n".join(MARKER_LINE_RE.findall(text))
        src = os.path.basename(path)
        for ref in REF_FEEDBACK.findall(candidates):
            scanned += 1
            if _canon_feedback(ref) not in have_feedback:
                missing.append("MISSING_REF: %s (%s) — feedback 백킹 파일 없음" % (ref, src))
        for ref in REF_BIN.findall(candidates):
            scanned += 1
            if ref not in have_bin:
                missing.append("MISSING_REF: %s (%s) — bin 도구 없음" % (ref, src))
        for ref in REF_DIRECTIVE.findall(candidates):
            scanned += 1
            if ref not in have_directive:
                missing.append("MISSING_REF: %s (%s) — directive 파일 없음" % (ref, src))
    return missing, scanned


def pack_dir():
    """pack 위치 결정 — src/pack.rs pack_dir()의 4단 폴백을 그대로 미러링한다."""
    for key in ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_JARVIS_DIR"):
        v = os.environ.get(key, "")
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys/pack")


def _find_codegen_kinds():
    """build.rs 산출 OUT_DIR/cys_kinds.json 을 crate target/ 아래에서 탐색(C44 3자 대조용).
    부재 시 None(빌드 미수행 — C44는 2자 대조로 강등). pack_dir의 부모(크레이트 루트) target/만
    훑어 무관 경로 배회를 피한다."""
    crate_root = os.path.dirname(os.path.abspath(pack_dir().rstrip("/")))
    target = os.path.join(crate_root, "target")
    if not os.path.isdir(target):
        return None
    for root, _dirs, files in os.walk(target):
        if "cys_kinds.json" in files:
            try:
                return json.load(open(os.path.join(root, "cys_kinds.json"), encoding="utf-8"))
            except Exception:
                continue
    return None


def _parse_skill_requires(text):
    """SKILL.md 프론트매터 `cys:` 블록의 requires.bins·install 매니페스트를 stdlib만으로 파싱.
    (C50 — preflight는 yaml 의존 0이므로 이 self-describing 스키마 하위셋만 손파싱한다.)
    반환: (bins|None, install_list). requires.bins 미선언이면 (None, []) — 스코프 밖 신호.
    지원 형태(이 스키마가 쓰는 표기만):
        requires:\n  bins: [ffmpeg, rg]
        install:\n  - {kind: brew, formula: ffmpeg}\n  - {kind: apt, package: ffmpeg}
    매니페스트 표기 일탈(예: bins 누락·install 비-인라인)은 호출부에서 fail-loud로 잡힌다."""
    bm = re.search(r"^\s*bins:\s*\[([^\]]*)\]", text, re.M)
    if not bm:
        return None, []
    bins = [b.strip().strip("'\"") for b in bm.group(1).split(",") if b.strip()]
    install = []
    for line in text.splitlines():
        em = re.match(r"\s*-\s*\{([^}]*)\}\s*$", line)
        if not em:
            continue
        entry = {}
        for pair in em.group(1).split(","):
            if ":" in pair:
                k, _, v = pair.partition(":")
                entry[k.strip().strip("'\"")] = v.strip().strip("'\"")
        if entry.get("kind") in ("brew", "apt"):
            install.append(entry)
    return bins, install


def is_dept_pack():
    """부서/CEO pack 컨텍스트인가 — pack_dir이 기본(~/.cys/pack)이 아니면 부서/CEO 데몬이다.
    부서장·CEO의 MASTER_DIRECTIVE는 표준 핀이 없는 게 정상이라 C03 표준 핀 검사를 면제한다
    (멀티마스터 정식화 F1 — 부서 운영 중 C03 영구 FAIL→`--force` 복원이 CEO 디렉티브를 파괴하는 것 차단)."""
    default = os.path.join(os.path.expanduser("~"), ".cys/pack")
    try:
        return os.path.realpath(pack_dir()) != os.path.realpath(default)
    except OSError:
        return False


def discover_claude_settings():
    """$HOME 직하 .claude*/settings.json 전부 (존재 파일만, 사전순) — cys.rs와 동일 규칙."""
    home = os.path.expanduser("~")
    found = []
    try:
        names = os.listdir(home)
    except OSError:
        return found
    for n in sorted(names):
        if n == ".claude" or n.startswith(".claude-"):
            p = os.path.join(home, n, "settings.json")
            if os.path.isfile(p):
                found.append(p)
    return found


class Preflight:
    def __init__(self, fix, skips):
        self.fix = fix
        self.skips = set(skips)
        self.results = []
        self._init_pack_ran = None  # None=미시도, True/False=시도 결과

    def add(self, cid, status, detail):
        self.results.append({"id": cid, "status": status, "detail": detail})

    def skipped(self, cid):
        if cid in self.skips:
            self.add(cid, SKIP, "skipped by --skip")
            return True
        return False

    # ── 공용 수리: cys init-pack (누락 템플릿만 재설치 — 사용자 수정본 불가침) ──
    def repair_via_init_pack(self):
        if self._init_pack_ran is not None:
            return self._init_pack_ran
        cys = shutil.which("cys")
        if not cys:
            self._init_pack_ran = False
            return False
        try:
            r = subprocess.run(
                [cys, "init-pack", "--no-install-hook"],
                capture_output=True, timeout=30,
            )
            self._init_pack_ran = r.returncode == 0
        except Exception:
            self._init_pack_ran = False
        return self._init_pack_ran

    # ── C01 pack 디렉터리 ──
    def c01_pack_dir(self):
        cid = "C01.pack-dir"
        if self.skipped(cid):
            return
        d = pack_dir()
        if os.path.isdir(d):
            self.add(cid, PASS, d)
            return
        if self.fix and self.repair_via_init_pack() and os.path.isdir(d):
            self.add(cid, FIXED, "%s (cys init-pack로 생성)" % d)
            return
        self.add(cid, FAIL, "%s 없음 — `cys init-pack` 실행 필요" % d)

    # ── C02 디렉티브 4종 존재·비어있지 않음 ──
    def c02_directives(self):
        cid = "C02.directives"
        if self.skipped(cid):
            return
        missing = []
        for f in DIRECTIVES:
            p = os.path.join(pack_dir(), "directives", f)
            if not (os.path.isfile(p) and os.path.getsize(p) > 0):
                missing.append(f)
        if missing and self.fix and self.repair_via_init_pack():
            missing = [
                f for f in missing
                if not os.path.isfile(os.path.join(pack_dir(), "directives", f))
            ]
            if not missing:
                self.add(cid, FIXED, "누락 디렉티브 재설치 완료")
                return
        if missing:
            self.add(cid, FAIL, "누락/빈 파일: %s" % ", ".join(missing))
        else:
            self.add(cid, PASS, "4종 디렉티브 존재·비공백")

    # ── C03 내용 핀 (절대지침 조항이 문서에 살아있는가) ──
    def c03_content_pins(self):
        if is_dept_pack():
            self.add("C03.pin", WARN,
                     "부서/CEO pack(%s) — 표준 디렉티브 핀 검사 면제(CEO/부서장 커스텀 디렉티브가 정상)"
                     % pack_dir())
            return
        for f, pins in CONTENT_PINS.items():
            cid = "C03.pin.%s" % f.split("_")[0].lower()
            if self.skipped(cid):
                continue
            p = os.path.join(pack_dir(), "directives", f)
            try:
                text = open(p, encoding="utf-8", errors="replace").read()
            except OSError:
                self.add(cid, FAIL, "%s 읽기 불가 (C02 먼저 해결)" % f)
                continue
            lost = [label for pin, label in pins if pin not in text]
            if lost:
                self.add(
                    cid, FAIL,
                    "%s에서 소실된 조항: %s — 템플릿 복원은 `cys init-pack --force`"
                    "(사용자 수정 덮어씀, 오너 결정 필요)" % (f, "; ".join(lost)),
                )
            else:
                self.add(cid, PASS, "%s 핀 %d개 전부 존재" % (f, len(pins)))

    # ── C04 soul.md 호칭 규정 ──
    def c04_soul(self):
        cid = "C04.soul"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "soul.md")
        if not os.path.isfile(p):
            if self.fix and self.repair_via_init_pack() and os.path.isfile(p):
                pass  # 재설치됨 — 아래 호칭 검사로 계속
            else:
                self.add(cid, FAIL, "soul.md 없음")
                return
        text = open(p, encoding="utf-8", errors="replace").read()
        # 2개 정책 마커: ①오너 호칭 ②자율주행 위임권(앵커6 — soul이 권한을 부여해야
        # MASTER §14 발효). 둘 다 --fix로 기본 골격을 보강할 수 있다(내용은 오너 주권).
        fixed = []
        if SOUL_MARKER not in text:
            if not self.fix:
                self.add(cid, FAIL,
                         "soul.md에 '오너 호칭' 규정 부재 — --fix로 기본값(주인님) 보강 가능")
                return
            if SOUL_PLACEHOLDER in text:
                text = text.replace(
                    SOUL_PLACEHOLDER,
                    '(이름을 적어라)\n- **오너 호칭: master는 오너를 "주인님"으로 호칭한다** (수정 가능)',
                    1,
                )
            else:
                text += SOUL_APPEND
            fixed.append("호칭 규정(기본 주인님)")
        # 자율주행 절은 '권한 부여 조항'이라 --fix가 자동 재주입하지 않는다(적대 검증 6차
        # H-2: 오너가 권한 회수 의사로 절을 삭제하면 다음 부트의 의무 --fix가 권한을 자동
        # 복원해 "절 부재=자율주행 안 함" 상태가 도달 불가능해진다 — 부여 주체는 오너뿐).
        # 절 부재는 유효한 '자율주행 비활성' 상태 — WARN으로 알리고 부트는 막지 않는다.
        autopilot_note = ""
        if SOUL_AUTOPILOT_MARKER not in text:
            autopilot_note = (" · 자율주행 위임권 절 부재 — 자율주행 비활성(MASTER §14 미발효). "
                              "부여하려면 오너가 soul.md에 절을 직접 추가하라"
                              "(표준 문안: 이 스크립트의 SOUL_AUTOPILOT_TEMPLATE)")
        if fixed:
            open(p, "w", encoding="utf-8").write(text)
            self.add(cid, FIXED, "soul.md 보강: %s%s" % (", ".join(fixed), autopilot_note))
        elif autopilot_note:
            self.add(cid, WARN, "호칭 규정 존재%s" % autopilot_note)
        else:
            self.add(cid, PASS, "호칭 규정 + 자율주행 위임권 절 존재 (내용은 오너 주권)")

    # ── 공용 수리: 파손 JSON을 백업 후 템플릿으로 복원 ──
    # 파싱이 죽은 파일은 '유효한 사용자 수정'이 아니다 — .broken 백업을 남기고
    # init-pack 템플릿으로 되살리는 것이 안전한 결정론 수리다(내용 손실 없음: 백업 보존).
    def restore_broken_json(self, path):
        if not self.fix:
            return False
        if os.path.islink(path):
            return False  # symlink 거부 — 링크 너머 실파일 훼손 차단(TOCTOU 방어)
        try:
            if os.path.isfile(path):
                shutil.move(path, path + ".broken-preflight")
        except OSError:
            return False
        self._init_pack_ran = None  # 파일을 치웠으니 재시도 허용
        return self.repair_via_init_pack() and os.path.isfile(path)

    # ── C05 agents.json 역할 매핑 ──
    def c05_agents(self):
        cid = "C05.agents-json"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "agents.json")
        if not os.path.isfile(p) and self.fix and self.repair_via_init_pack():
            pass
        fixed_broken = False
        try:
            data = json.load(open(p, encoding="utf-8"))
        except (OSError, ValueError) as e:
            if self.restore_broken_json(p):
                try:
                    data = json.load(open(p, encoding="utf-8"))
                    fixed_broken = True
                except (OSError, ValueError) as e2:
                    self.add(cid, FAIL, "agents.json 복원 후에도 파싱 실패: %s" % e2)
                    return
            else:
                self.add(cid, FAIL, "agents.json 파싱 실패: %s — --fix로 백업·복원 가능" % e)
                return
        problems = []
        for a in ("claude", "gemini", "codex"):
            if not isinstance(data.get(a), dict) or "cmd" not in data[a]:
                problems.append("어댑터 %s 누락/불완전" % a)
        roles = data.get("_roles", {})
        for r in ROLES:
            f = roles.get(r)
            if not f:
                problems.append("_roles.%s 매핑 누락" % r)
            elif not os.path.isfile(os.path.join(pack_dir(), f)):
                problems.append("_roles.%s → %s 파일 없음" % (r, f))
        if problems:
            self.add(cid, FAIL, "; ".join(problems))
        elif fixed_broken:
            self.add(cid, FIXED, "파손 agents.json 백업(.broken-preflight) 후 템플릿 복원")
        else:
            self.add(cid, PASS, "어댑터 3종 + 역할 매핑 4종 정합")

    # ── C06 acl.json / schedule.json 파싱 ──
    def c06_json_files(self):
        cid = "C06.json-parse"
        if self.skipped(cid):
            return
        problems = []
        fixed = []
        for f in ("acl.json", "schedule.json"):
            p = os.path.join(pack_dir(), f)
            if not os.path.isfile(p):
                if self.fix and self.repair_via_init_pack() and os.path.isfile(p):
                    pass
                else:
                    problems.append("%s 없음" % f)
                    continue
            try:
                json.load(open(p, encoding="utf-8"))
            except (OSError, ValueError) as e:
                if self.restore_broken_json(p):
                    try:
                        json.load(open(p, encoding="utf-8"))
                        fixed.append(f)
                        continue
                    except (OSError, ValueError):
                        pass
                problems.append("%s 파싱 실패: %s" % (f, e))
        if problems:
            self.add(cid, FAIL, "; ".join(problems))
        elif fixed:
            self.add(cid, FIXED, "파손 복원: %s (.broken-preflight 백업)" % ", ".join(fixed))
        else:
            self.add(cid, PASS, "acl.json·schedule.json 정상")

    # ── C07 hook 스크립트 존재·실행권한 ──
    def c07_hook_script(self):
        cid = "C07.hook-script"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "hooks", "session-start.sh")
        if not os.path.isfile(p):
            if self.fix and self.repair_via_init_pack() and os.path.isfile(p):
                pass
            else:
                self.add(cid, FAIL, "hooks/session-start.sh 없음")
                return
        if os.name == "posix":
            mode = os.stat(p).st_mode
            if not mode & stat.S_IXUSR:
                if self.fix:
                    os.chmod(p, mode | 0o755)
                    self.add(cid, FIXED, "실행권한 부여(755)")
                    return
                self.add(cid, WARN, "실행권한 없음 (sh 명시 호출이라 동작은 하나 권장 755)")
                return
        self.add(cid, PASS, p)

    # ── C08 SessionStart hook 등록 (Claude 설정) ──
    def _hook_registered(self, settings_path):
        try:
            data = json.load(open(settings_path, encoding="utf-8"))
        except (OSError, ValueError):
            return False
        marker = os.path.join("hooks", "session-start.sh")
        for entry in data.get("hooks", {}).get("SessionStart", []):
            for h in entry.get("hooks", []):
                cmd = h.get("command", "")
                if marker in cmd and "pack" in cmd:
                    return True
        return False

    def _register_hook(self, settings_path):
        """hook 등록. 성공=None, 실패=사유 문자열 (호출자가 FAIL로 보고).

        안전장치: ①symlink 거부(링크 너머 실파일 훼손 차단) ②기존 파일이 JSON으로
        파싱 안 되면 {}로 대체하지 않고 거부 — 침묵 데이터 소실 차단(rust 구현과 동일 규약).
        """
        if os.path.islink(settings_path):
            return "symlink 거부(실파일만 허용): %s" % settings_path
        script = os.path.join(pack_dir(), "hooks", "session-start.sh")
        cmd = ("bash " if os.name == "nt" else "sh ") + script
        if os.path.isfile(settings_path):
            try:
                data = json.load(open(settings_path, encoding="utf-8"))
            except (OSError, ValueError) as e:
                return ("기존 settings.json 파싱 실패 — 덮어쓰기 거부(수동 복구 필요): %s (%s)"
                        % (settings_path, e))
            if not isinstance(data, dict):
                return "settings.json 루트가 객체가 아님 — 거부: %s" % settings_path
            # 최초 백업만 보존 — 재실행이 정상 백업을 손상 상태로 덮어쓰는 것을 차단.
            backup = settings_path + ".bak-preflight"
            if not os.path.exists(backup):
                shutil.copy2(settings_path, backup)
        else:
            data = {}
            d = os.path.dirname(settings_path)
            if d:
                os.makedirs(d, exist_ok=True)
        arr = data.setdefault("hooks", {}).setdefault("SessionStart", [])
        arr.append({"hooks": [{"type": "command", "command": cmd}]})
        # 원자적 쓰기(tmp+replace) — truncate-write 중 크래시가 settings.json을
        # 파손시키면 다음 실행이 파싱 거부로 수리 불능에 빠진다(전수조사 발견).
        tmp = settings_path + ".tmp"
        open(tmp, "w", encoding="utf-8").write(
            json.dumps(data, ensure_ascii=False, indent=2)
        )
        os.replace(tmp, settings_path)
        return None

    def c08_hook_registered(self):
        cid = "C08.hook-registered"
        if self.skipped(cid):
            return
        targets = discover_claude_settings()
        if not targets:
            default = os.path.join(os.path.expanduser("~"), ".claude", "settings.json")
            if self.fix:
                err = self._register_hook(default)
                if err:
                    self.add(cid, FAIL, err)
                else:
                    self.add(cid, FIXED, "Claude 설정 미발견 → %s 생성·등록" % default)
            else:
                self.add(cid, FAIL, "~/.claude*/settings.json 미발견 — --fix로 생성 가능")
            return
        unregistered = [t for t in targets if not self._hook_registered(t)]
        if not unregistered:
            self.add(cid, PASS, "%d개 프로필 전부 hook 등록됨" % len(targets))
            return
        if self.fix:
            done, errs = [], []
            for t in unregistered:
                err = self._register_hook(t)
                if err:
                    errs.append(err)
                else:
                    done.append(t)
            if errs:
                self.add(cid, FAIL, "; ".join(errs)
                         + (" | 등록 성공: %s" % ", ".join(done) if done else ""))
            else:
                self.add(cid, FIXED, "hook 등록: %s" % ", ".join(done))
        else:
            self.add(cid, FAIL, "hook 미등록 프로필: %s" % ", ".join(unregistered))

    # ── C32 statusline 래퍼 등록 (Claude 설정 — T5 Phase 2-A claude rate limit 채널) ──
    # claude의 5h/주간 rate limit 잔량은 로컬 파일에 없다 — 유일한 무간섭 채널이 statusline
    # stdin JSON이다. cys-statusline.sh가 매 메시지마다 usage.report로 push해 pane 배지에
    # 5h/7d를 띄운다. 부가 기능이라(ctx 배지는 없어도 작동) 미설치는 WARN(READY 미차단).
    def _statusline_registered(self, settings_path):
        try:
            data = json.load(open(settings_path, encoding="utf-8"))
        except (OSError, ValueError):
            return False
        sl = data.get("statusLine")
        return isinstance(sl, dict) and "cys-statusline.sh" in sl.get("command", "")

    def _register_statusline(self, settings_path):
        """statusLine 등록. 성공=None, 실패=사유 문자열. 기존 statusLine은 CYS_PREV_STATUSLINE로
        래핑해 체인 보존(덮어쓰기 금지) — _register_hook과 동일한 symlink 거부·파싱 거부·최초
        백업·원자적 쓰기 철학."""
        if os.path.islink(settings_path):
            return "symlink 거부(실파일만 허용): %s" % settings_path
        script = os.path.join(pack_dir(), "hooks", "cys-statusline.sh")
        runner = "bash " if os.name == "nt" else "sh "
        if os.path.isfile(settings_path):
            try:
                data = json.load(open(settings_path, encoding="utf-8"))
            except (OSError, ValueError) as e:
                return ("기존 settings.json 파싱 실패 — 덮어쓰기 거부(수동 복구 필요): %s (%s)"
                        % (settings_path, e))
            if not isinstance(data, dict):
                return "settings.json 루트가 객체가 아님 — 거부: %s" % settings_path
            backup = settings_path + ".bak-preflight"
            if not os.path.exists(backup):
                shutil.copy2(settings_path, backup)
        else:
            data = {}
            d = os.path.dirname(settings_path)
            if d:
                os.makedirs(d, exist_ok=True)
        # 기존 statusLine(우리 것이 아니면) → CYS_PREV_STATUSLINE로 보존 체인(사람용 줄 위임).
        prev = data.get("statusLine")
        prev_cmd = prev.get("command", "") if isinstance(prev, dict) else ""
        if prev_cmd and "cys-statusline.sh" not in prev_cmd:
            cmd = "CYS_PREV_STATUSLINE=%s %s%s" % (shlex.quote(prev_cmd), runner, script)
        else:
            cmd = runner + script
        data["statusLine"] = {"type": "command", "command": cmd}
        tmp = settings_path + ".tmp"
        open(tmp, "w", encoding="utf-8").write(
            json.dumps(data, ensure_ascii=False, indent=2)
        )
        os.replace(tmp, settings_path)
        return None

    def c32_statusline(self):
        cid = "C32.statusline"
        if self.skipped(cid):
            return
        script = os.path.join(pack_dir(), "hooks", "cys-statusline.sh")
        if not os.path.isfile(script):
            if not (self.fix and self.repair_via_init_pack() and os.path.isfile(script)):
                self.add(cid, FAIL, "hooks/cys-statusline.sh 없음 — `cys init-pack` 또는 --fix")
                return
        targets = discover_claude_settings()
        if not targets:
            self.add(cid, WARN, "~/.claude*/settings.json 미발견 — claude 노드 기동 후 재실행")
            return
        unregistered = [t for t in targets if not self._statusline_registered(t)]
        if not unregistered:
            self.add(cid, PASS,
                     "%d개 프로필 statusLine 등록됨 (claude 재시작 후 5h/7d rate limit 배지 적용)"
                     % len(targets))
            return
        if self.fix:
            done, errs = [], []
            for t in unregistered:
                err = self._register_statusline(t)
                errs.append(err) if err else done.append(t)
            if errs:
                self.add(cid, FAIL, "; ".join(errs)
                         + (" | 등록 성공: %s" % ", ".join(done) if done else ""))
            else:
                self.add(cid, FIXED, "statusLine 등록: %s — ★claude 재시작 후 적용" % ", ".join(done))
        else:
            self.add(cid, WARN, "statusLine 미등록 프로필: %s — --fix로 설치(claude 재시작 후 적용)"
                     % ", ".join(unregistered))

    # ── C33 툴 이벤트 hook (T7 E1-④ — events 테이블 적재) ──
    # PreToolUse/PostToolUse에 hooks/cys-hook.sh 등록 → 툴·스킬·에이전트 호출·exit_code를
    # cysd events 테이블에 적재(E3 스킬 TOP·반복실패 토대). hook은 fail-open(에이전트 무차단)이라
    # 무관 작업 불간섭. C32와 동일 규약(체인보존·symlink/파손 거부·원자적). FAIL 없음(미등록=WARN).
    EVENT_HOOK = "cys-hook.sh"
    EVENT_HOOK_EVENTS = ("PreToolUse", "PostToolUse")

    def c33_event_hooks(self):
        cid = "C33.event-hooks"
        if self.skipped(cid):
            return
        script = os.path.join(pack_dir(), "hooks", self.EVENT_HOOK)
        if not os.path.isfile(script):
            if not (self.fix and self.repair_via_init_pack() and os.path.isfile(script)):
                self.add(cid, WARN, "hooks/%s 없음 — `cys init-pack` 또는 --fix" % self.EVENT_HOOK)
                return
        targets = discover_claude_settings()
        if not targets:
            self.add(cid, WARN, "~/.claude*/settings.json 미발견 — claude 노드 기동 후 재실행")
            return
        # 프로필×이벤트 단위로 미등록 항목 수집
        pending = [(t, ev) for t in targets for ev in self.EVENT_HOOK_EVENTS
                   if not self._event_hook_registered(t, ev, self.EVENT_HOOK)]
        if not pending:
            self.add(cid, PASS,
                     "%d개 프로필 PreToolUse/PostToolUse 이벤트 hook 등록됨 (claude 재시작 후 적용)"
                     % len(targets))
            return
        if self.fix:
            done, errs = [], []
            for t, ev in pending:
                err = self._register_event_hook(t, ev, self.EVENT_HOOK, matcher="")
                errs.append("%s/%s: %s" % (os.path.basename(os.path.dirname(t)), ev, err)) \
                    if err else done.append("%s/%s" % (os.path.basename(os.path.dirname(t)), ev))
            if errs:
                self.add(cid, WARN, "; ".join(errs)
                         + (" | 등록 성공: %s" % ", ".join(done) if done else ""))
            else:
                self.add(cid, FIXED, "이벤트 hook 등록: %s — ★claude 재시작 후 적용" % ", ".join(done))
        else:
            self.add(cid, WARN, "이벤트 hook 미등록: %d건 — --fix로 설치(claude 재시작 후 적용)"
                     % len(pending))

    # ── C41 참조 무결성 (T6-P1 — soul/directive inline 백틱 참조 dangling 검출) ──
    # C18(MEMORY.md 색인↔파일)·C39(skill frontmatter 고아)와 비중첩: C41은 본문 inline
    # 백틱/마커 named 참조라는 새 표면만 본다. 자동수리 없음(문서 내용은 오너·노드 소관).
    def c41_reference_integrity(self):
        cid = "C41.reference-integrity"
        if self.skipped(cid):
            return
        if is_dept_pack():  # 부서/CEO pack은 표준 문서 그래프 면제(C03와 동형)
            self.add(cid, WARN, "부서/CEO pack — 표준 참조 그래프 검사 면제")
            return
        missing, scanned = _scan_reference_integrity(pack_dir())
        if missing:
            self.add(cid, FAIL, "; ".join(sorted(set(missing)))
                     + " — 깨진 참조를 고치거나 누락 파일을 추가하라(자동수리 없음)")
        else:
            self.add(cid, PASS, "참조 %d개 전부 해소(soul+directive %d파일·feedback/bin/directive 3클래스)"
                     % (scanned, 1 + len(DIRECTIVES)))

    # ── C42 MPL 클린룸 가드레일 (T6-P6 — 흡수 연구 코드복사0 박제) ──
    # 도구 로직 정합은 javis_cleanroom.py --self-test에 위임(C17/C19 _check_bin_tool 패턴 동형).
    def c42_cleanroom(self):
        cid = "C42.cleanroom-guardrail"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_cleanroom.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (4원칙 키·마커쌍·라이선스 화이트리스트 검증)" % p)

    # ── C43 verdict 리터럴 단일진실 핀 (T1-1 — T6-P5 이후 schema 파생) ──
    # 측정 결과 경계를 건너는 #[repr(u8)] 판별값 enum = 0개(grep "repr(u" src/ = 0) →
    # build.rs enum→TS/JSON codegen은 빌드하지 않고(미래 착륙 조건만 문서화), verdict
    # 4-리터럴(판별값 없는 문자열 집합)의 손동기 드리프트만 문자열-동치로 fail-loud 차단한다.
    # ★T6-P5 이후: 코드측 리터럴이 verdict_schema.json(단일 머신-SOT)에서 파생되므로, C43 도
    #   소스-A 를 그 스키마의 reviewer_enum(=리뷰어가 채우는 4 enum, INVESTIGATE 제외)에서 읽는다.
    #   스키마 부재 시 graceful 폴백(javis_verdict.py 의 VERDICT_ENUM 그렙 — verdict.py 폴백과 정합).
    # ★C36/C51과 중복 아님: C36=verdict *인스턴스* 검증, C51=스키마↔코드 *로드/정합*, C43=리터럴이
    #   *계약 텍스트(REVIEWER_DIRECTIVE.md)* 와 동기인지 대조. 코드 무결성은 C36 self-test에 위임.
    def c43_verdict_literals(self):
        cid = "C43.verdict-literals"
        if self.skipped(cid):
            return
        vbin = os.path.join(pack_dir(), "bin", "javis_verdict.py")
        schema_p = os.path.join(pack_dir(), "schemas", "verdict_schema.json")
        # A = SOT측 리뷰어 4리터럴. 1차: verdict_schema.json reviewer_enum. 폴백: verdict.py 그렙.
        a = None
        src_label = ""
        if os.path.isfile(schema_p):
            try:
                s = json.load(open(schema_p, encoding="utf-8"))
                a = set(s.get("reviewer_enum", [])) or None
                src_label = "verdict_schema.json:reviewer_enum"
            except (OSError, ValueError):
                a = None
        if a is None:
            if not os.path.isfile(vbin):
                self.add(cid, WARN, "verdict_schema.json·javis_verdict.py 모두 부재 — "
                         "verdict 단일진실 검사 보류")
                return
            try:
                src = open(vbin, encoding="utf-8").read()
            except Exception as e:
                self.add(cid, WARN, "javis_verdict.py 읽기 불가 — 보류: %s" % e)
                return
            m = re.search(r"VERDICT_ENUM\s*=\s*\(([^)]*)\)", src)
            a = (set(re.findall(r'"([A-Z]+)"', m.group(1))) - {"INVESTIGATE"}) if m else set()
            src_label = "javis_verdict.py:32(폴백)"
        expect = {"ACCEPT", "REVISE", "BLOCK", "ESCALATE"}
        if a != expect:
            self.add(cid, FAIL, "%s 기대 4리터럴과 불일치: %s (기대 %s)"
                     % (src_label, sorted(a), sorted(expect)))
            return
        # B = 계약측 (pack 임베드 REVIEWER_DIRECTIVE.md 의 verdict 리터럴 텍스트)
        # 형태: `{verdict: ACCEPT|REVISE|BLOCK|ESCALATE, ...}` 또는 `"verdict": "ACCEPT | REVISE | ..."`
        contract = os.path.join(pack_dir(), "directives", "REVIEWER_DIRECTIVE.md")
        if not os.path.isfile(contract):
            self.add(cid, PASS, "verdict 4리터럴 self-consistent(%s) — "
                     "계약파일 부재로 교차검증 보류" % src_label)
            return
        try:
            ctext = open(contract, encoding="utf-8").read()
        except Exception as e:
            self.add(cid, WARN, "REVIEWER_DIRECTIVE.md 읽기 불가 — 보류: %s" % e)
            return
        cm = re.search(r"verdict[\"'\s:]*\s*([A-Z]+(?:\s*\|\s*[A-Z]+)+)", ctext)
        if not cm:
            self.add(cid, PASS, "verdict 4리터럴 self-consistent(%s) — "
                     "계약 verdict 텍스트 미검출로 교차검증 보류" % src_label)
            return
        b = set(re.findall(r"[A-Z]+", cm.group(1)))
        if a == b:
            self.add(cid, PASS, "verdict 4리터럴 손동기 일치(SOT:%s ↔ "
                     "계약:REVIEWER_DIRECTIVE.md)" % src_label)
        else:
            self.add(cid, FAIL, "verdict 드리프트 — 코드만:%s / 계약만:%s"
                     % (sorted(a - b), sorted(b - a)))

    # ── C44 kind/mode/transition enum 파리티 (T1-2 — 다중 출력 경로 누락0·드리프트0 FLOOR) ──
    # cys 다중 소비 경로가 공유하는 "종류 집합"의 단일진실 동기화를 결정론 검사한다:
    #   schema(edit_decisions.schema.json) ↔ check_timeline.py 상수(TRACK_KINDS/EL_MODES/EL_TRANSITIONS).
    # build.rs 산출 cys_kinds.json(OUT_DIR)이 보이면 3자, 안 보이면 2자(schema↔check_timeline) 대조.
    # ★C43과 비충돌(별 cid·verdict 도메인 무관). 과신 금지: 누락0/드리프트0의 FLOOR이지 프레임 패리티
    # 보장이 아니다(프레임 패리티는 별도 raster-diff 하네스 소관).
    def c44_kind_enum_parity(self):
        cid = "C44.kind-enum-parity"
        if self.skipped(cid):
            return
        schema_p = os.path.join(pack_dir(), "schemas", "edit_decisions.schema.json")
        ct_p = os.path.join(pack_dir(), "bin", "check_timeline.py")
        if not os.path.isfile(schema_p) or not os.path.isfile(ct_p):
            self.add(cid, WARN, "edit_decisions.schema.json 또는 check_timeline.py 부재 — "
                     "enum 파리티 검사 보류")
            return
        try:
            schema = json.load(open(schema_p, encoding="utf-8"))
            ctext = open(ct_p, encoding="utf-8").read()
        except Exception as e:
            self.add(cid, WARN, "schema/check_timeline 읽기 불가 — 보류: %s" % e)
            return
        # schema 측 3개 enum(경로 고정 — 스키마 구조에 박제).
        try:
            s_kind = set(schema["properties"]["tracks"]["items"]["properties"]["kind"]["enum"])
            s_mode = set(schema["$defs"]["element"]["properties"]["mode"]["enum"])
            s_trans = set(schema["$defs"]["element"]["properties"]["transition"]["enum"])
        except (KeyError, TypeError) as e:
            self.add(cid, FAIL, "edit_decisions.schema.json enum 경로 변형 — %s" % e)
            return
        # check_timeline 측 상수(C43과 동형 regex 파싱 — import 부작용 회피).
        def _tuple_literals(name):
            m = re.search(name + r'\s*=\s*\(([^)]*)\)', ctext)
            return set(re.findall(r'"([^"]+)"', m.group(1))) if m else None
        ct_kind = _tuple_literals("TRACK_KINDS")
        ct_mode = _tuple_literals("EL_MODES")
        ct_trans = _tuple_literals("EL_TRANSITIONS")
        if ct_kind is None or ct_mode is None or ct_trans is None:
            self.add(cid, FAIL, "check_timeline.py TRACK_KINDS/EL_MODES/EL_TRANSITIONS 미검출")
            return
        for label, sset, cset in (("kind", s_kind, ct_kind),
                                  ("mode", s_mode, ct_mode),
                                  ("transition", s_trans, ct_trans)):
            if sset != cset:
                self.add(cid, FAIL, "%s enum 드리프트 — schema만:%s / check_timeline만:%s"
                         % (label, sorted(sset - cset), sorted(cset - sset)))
                return
        # cys_kinds.json(build.rs 산출) 발견 시 3자 대조(없으면 2자로 PASS·빌드 후 재검 안내).
        gen = _find_codegen_kinds()
        if gen is None:
            self.add(cid, PASS, "kind/mode/transition enum 2자 일치(schema ↔ check_timeline) — "
                     "cys_kinds.json 미발견(빌드 후 3자 재검 가능)")
            return
        if (set(gen.get("edit_kind", [])) == s_kind
                and set(gen.get("mode", [])) == s_mode
                and set(gen.get("transition", [])) == s_trans):
            self.add(cid, PASS, "kind/mode/transition enum 3자 일치"
                     "(cys_kinds.json ↔ schema ↔ check_timeline)")
        else:
            self.add(cid, FAIL, "cys_kinds.json(build.rs codegen)이 schema/check_timeline과 드리프트")

    # ── C45 state-db change-log 무결성 (T2-3 — append-only change-log + 단조 revn) ──
    # 라이브 analytics.db(부재 시 graceful WARN)에서 change-log 스키마 존재 + revn 단조성을
    # 결정론 확인한다. 스키마는 analytics.rs open()이 ADDITIVE로 보장(SESSION_STATE.md 산문
    # 복원 경로는 불변 — 이 체크는 그 위에 얹은 change-replay 능력층의 무결성만 본다).
    # ★owner c38=silent_failure_catalog이므로 stale spec ref "c38_statedb" 대신 신규 C45 사용.
    def c45_statedb(self):
        cid = "C45.state-db"
        if self.skipped(cid):
            return
        import sqlite3
        # analytics.db 위치: CYS_SOCKET 부모(데몬 state_dir) → 기본 ~/.local/state/cys.
        sock = os.environ.get("CYS_SOCKET") or os.environ.get("JAVIS_SOCKET") \
            or os.environ.get("AITERM_SOCKET")
        if sock:
            db = os.path.join(os.path.dirname(sock), "analytics.db")
        else:
            db = os.path.join(os.path.expanduser("~"), ".local", "state", "cys", "analytics.db")
        if not os.path.isfile(db):
            self.add(cid, WARN, "analytics.db 부재(데몬 미기동/미생성) — change-log 무결성 검사 보류")
            return
        try:
            conn = sqlite3.connect("file:%s?mode=ro" % db, uri=True)
        except sqlite3.Error as e:
            self.add(cid, WARN, "analytics.db 열기 실패 — 보류: %s" % e)
            return
        try:
            tabs = {r[0] for r in conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table'")}
            need = {"state_scope", "change_log"}
            if not need <= tabs:
                self.add(cid, FAIL, "change-log 스키마 부재: %s (analytics.rs open() 갱신 필요)"
                         % ", ".join(sorted(need - tabs)))
                return
            # revn 단조성: 각 scope에서 seq 오름차순일 때 revn이 단조 비감소여야 한다.
            bad = []
            scopes = [r[0] for r in conn.execute("SELECT DISTINCT scope FROM change_log")]
            for sc in scopes:
                prev = None
                for (revn,) in conn.execute(
                        "SELECT revn FROM change_log WHERE scope=? ORDER BY seq ASC", (sc,)):
                    if prev is not None and revn < prev:
                        bad.append(sc)
                        break
                    prev = revn
            if bad:
                self.add(cid, FAIL, "revn 단조성 위반 scope: %s" % ", ".join(bad))
                return
            self.add(cid, PASS, "change-log 스키마 존재 + revn 단조성 정합(scope %d개)" % len(scopes))
        except sqlite3.Error as e:
            self.add(cid, WARN, "change-log 조회 실패 — 보류: %s" % e)
        finally:
            conn.close()

    # ── C46 SESSION_STATE 예약 엔티티 ensure (T3-5 — ensure-X 복원 불변식) ──
    def c46_session_ensure(self):
        cid = "C46.session-ensure"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "bin", "javis_session.py")
        if not os.path.isfile(p):
            self.add(cid, WARN, "javis_session.py 부재 — 예약 엔티티 검사 보류")
            return
        # bin 무결성: 그 bin의 --self-test를 subprocess로(c36/c43 패턴).
        st = subprocess.run([sys.executable, p, "--self-test"], capture_output=True)
        if st.returncode != 0:
            self.add(cid, FAIL, "javis_session.py --self-test 실패(배터리 불통과)")
            return
        # producer≠evaluator: ensure 결과 파일을 verify로 독립 채점.
        ss = os.path.join(pack_dir(), "round", "SESSION_STATE.md")
        rc = subprocess.run([sys.executable, p, "verify", "--file", ss], capture_output=True).returncode
        if rc != 0 and self.fix:
            subprocess.run([sys.executable, p, "ensure", "--file", ss], capture_output=True)
            rc = subprocess.run([sys.executable, p, "verify", "--file", ss], capture_output=True).returncode
            if rc == 0:
                self.add(cid, FIXED, "예약 필드 ensure 복구")
                return
        if rc == 0:
            self.add(cid, PASS, "restore_pointer·open_gates 예약 엔티티 보장 + self-test ok")
        else:
            self.add(cid, FAIL, "예약 필드 부재/마커 깨짐 — --fix로 ensure")

    # ── C47 능력 가드 (T4-4/T6-P3 — producer≠evaluator 물리 경화) ──
    # 정적 검사: ① cysd 원장/surface caps 스키마(caps.rs Cap enum + state.rs caps 필드 +
    # write⊇read 정규화)가 존재하는가 ② PreToolUse hook(role-capability-gate.sh)이 reviewer
    # 역할 변형 도구(Edit/Write/NotebookEdit/Bash-write) 차단으로 배선됐고 self-test가 통과하는가
    # ③ 그 hook이 프로필 settings.json hooks.PreToolUse 에 **실제 등록**됐는가(파일존재+self-test만으론
    #    DORMANT 미배선을 못 잡는다 — 미등록이면 FAIL + `--fix` 힌트, --fix면 자동 배선).
    # caps 스키마/hook self-test는 라이브 데몬 불요. 부재 시 FAIL(보안 게이트 = 강한 단정).
    def c47_capability_guard(self):
        cid = "C47.capability-guard"
        if self.skipped(cid):
            return
        crate_root = os.path.dirname(os.path.abspath(pack_dir().rstrip("/")))
        caps_rs = os.path.join(crate_root, "src", "bin", "cysd", "caps.rs")
        state_rs = os.path.join(crate_root, "src", "bin", "cysd", "state.rs")
        handlers_rs = os.path.join(crate_root, "src", "bin", "cysd", "handlers.rs")
        hook = os.path.join(pack_dir(), "hooks", "role-capability-gate.sh")
        # ① 원장 caps 스키마 — 소스 정적 핀.
        if not os.path.isfile(caps_rs):
            self.add(cid, FAIL, "caps.rs 부재 — 원장 capability 스키마 미구현 (%s)" % caps_rs)
            return
        try:
            caps_src = open(caps_rs, encoding="utf-8").read()
            state_src = open(state_rs, encoding="utf-8").read() if os.path.isfile(state_rs) else ""
            handlers_src = open(handlers_rs, encoding="utf-8").read() if os.path.isfile(handlers_rs) else ""
        except OSError as e:
            self.add(cid, WARN, "cysd 소스 읽기 불가 — 보류: %s" % e)
            return
        need_caps = [
            ("Cap enum", "pub enum Cap"),
            ("write⊇read 정규화", "normalize_write_implies_read"),
            ("reviewer/planner 식별", "is_reviewer_or_planner"),
            ("deny-by-default none()", "fn none()"),
        ]
        miss = [label for label, tok in need_caps if tok not in caps_src]
        if "pub caps: Option<crate::caps::Caps>" not in state_src and "caps: Option<crate::caps::Caps>" not in state_src:
            miss.append("LedgerEntry.caps 필드")
        if "check_caps_gate" not in handlers_src:
            miss.append("handlers cysd-매개 게이트(check_caps_gate)")
        if miss:
            self.add(cid, FAIL, "capability 스키마/게이트 미배선: %s" % ", ".join(miss))
            return
        # ② PreToolUse hook 배선 — reviewer 변형 도구 denylist + self-test.
        if not os.path.isfile(hook):
            self.add(cid, FAIL, "role-capability-gate.sh(PreToolUse 물리 enforcer) 부재 (%s)" % hook)
            return
        try:
            hook_src = open(hook, encoding="utf-8").read()
        except OSError as e:
            self.add(cid, WARN, "hook 읽기 불가 — 보류: %s" % e)
            return
        hook_miss = [label for label, tok in (
            ("MUTATION_TOOLS denylist", "MUTATION_TOOLS"),
            ("reviewer/planner 판정", "is_reviewer_or_planner"),
            ("Edit 차단", '"Edit"'),
            ("write-shell 차단", "WRITE_SHELL_CMDS"),
        ) if tok not in hook_src]
        if hook_miss:
            self.add(cid, FAIL, "hook reviewer denylist 미배선: %s" % ", ".join(hook_miss))
            return
        st = subprocess.run(["bash", hook, "--self-test"], capture_output=True)
        if st.returncode != 0:
            detail = (st.stderr or st.stdout).decode("utf-8", "replace").strip()[:200]
            self.add(cid, FAIL, "role-capability-gate.sh --self-test 실패: %s" % detail)
            return
        # ③ PreToolUse 실제 등록 검증 — 파일 존재+self-test만으론 'DORMANT(미배선)'를 못 잡는다.
        #    프로필 settings.json hooks.PreToolUse 에 role-capability-gate가 들어있어야 PASS.
        #    --fix 면 자동 등록(appbuild-gate 등록과 동형·멱등).
        targets = discover_claude_settings()
        if not targets:
            # 프로필 미발견 — 라이브 wiring은 claude 노드 기동 후. 게이트 자체는 self-test로 검증됨.
            self.add(cid, WARN, "caps 스키마+cysd 게이트+hook self-test OK이나 ~/.claude*/settings.json "
                     "미발견 — claude 노드 기동 후 `preflight --fix`로 PreToolUse 등록 필요")
            return
        registered = [t for t in targets if self._capgate_hook_registered(t)]
        unwired = [t for t in targets if t not in registered]
        if unwired and self.fix:
            done, errs = [], []
            for t in unwired:
                err = self._register_capgate_hook(t)
                errs.append("%s: %s" % (os.path.basename(os.path.dirname(t)), err)) if err \
                    else done.append(os.path.basename(os.path.dirname(t)))
            if errs:
                self.add(cid, FAIL, "능력 GATE hook PreToolUse 등록 실패: %s%s"
                         % ("; ".join(errs),
                            " | 성공: %s" % ", ".join(done) if done else ""))
                return
            unwired = []  # 전부 등록됨
        if unwired:
            # 파일·self-test는 OK이고 등록만 남았다(deploy 시 --fix가 배선) → WARN+힌트로 보고하되
            # 게이트 자체 결함과 구분한다(스키마/hook 결함은 위에서 이미 FAIL). C45/C10 외 신규 FAIL 방지.
            self.add(cid, WARN, "role-capability-gate.sh 파일·self-test OK이나 PreToolUse 미배선"
                     "(DORMANT) — %d개 프로필 미등록. `preflight --fix`로 배선(claude 재시작 후 적용)"
                     % len(unwired))
            return
        self.add(cid, PASS, "원장 caps 스키마(Cap·write⊇read·deny-by-default) + cysd 게이트 + "
                 "PreToolUse hook reviewer denylist(self-test ok) + %d개 프로필 PreToolUse 등록 확인"
                 % len(targets))

    # ── C48 거버넌스 경화 (Wave3 UNIT B — watchdog 무음크래시·바이트상한·오염격리·좀비) ──
    # 정적 핀(라이브 데몬 불요): 네 가닥이 소스에 배선됐는지 토큰 존재로 박제한다.
    # ① T5-6 strand-2 ProcessHealth{Reusable,Poisoned} + LedgerEntry.health + is_reusable
    # ② T4-5A 단일 RPC 응답 바이트상한(MAX_RESPONSE_BYTES + cap_response, ONE guard)
    # ③ T5-2 surface_crashed 술어 + check_surface_crash watchdog 결선 + 재진입 가드
    # ④ T4-5B reap_zombie_surfaces watchdog 결선(3-miss 임계). 부재 시 FAIL(거버넌스 회귀).
    def c48_governance_hardening(self):
        cid = "C48.governance-hardening"
        if self.skipped(cid):
            return
        crate_root = os.path.dirname(os.path.abspath(pack_dir().rstrip("/")))
        cysd = os.path.join(crate_root, "src", "bin", "cysd")
        state_rs = os.path.join(cysd, "state.rs")
        gov_rs = os.path.join(cysd, "governance.rs")
        wire_rs = os.path.join(crate_root, "src", "wire.rs")
        try:
            state_src = open(state_rs, encoding="utf-8").read() if os.path.isfile(state_rs) else ""
            gov_src = open(gov_rs, encoding="utf-8").read() if os.path.isfile(gov_rs) else ""
            wire_src = open(wire_rs, encoding="utf-8").read() if os.path.isfile(wire_rs) else ""
        except OSError as e:
            self.add(cid, WARN, "cysd 소스 읽기 불가 — 보류: %s" % e)
            return
        miss = []
        # ① 오염 격리
        if "pub enum ProcessHealth" not in state_src or "Poisoned" not in state_src:
            miss.append("ProcessHealth{Reusable,Poisoned}")
        if "pub health: ProcessHealth" not in state_src:
            miss.append("LedgerEntry.health 필드")
        if "fn is_reusable" not in state_src:
            miss.append("is_reusable 재사용 술어")
        if "poison_surface_ledger" not in gov_src:
            miss.append("poison_surface_ledger 마킹")
        # ② 바이트상한 (ONE guard)
        if "MAX_RESPONSE_BYTES" not in wire_src or "fn cap_response" not in wire_src:
            miss.append("MAX_RESPONSE_BYTES/cap_response 바이트상한")
        # ③ 무음 크래시
        if "fn surface_crashed" not in gov_src:
            miss.append("surface_crashed 술어")
        if "fn check_surface_crash" not in gov_src or "check_surface_crash(&daemon)" not in gov_src:
            miss.append("check_surface_crash watchdog 결선")
        if "CRASH_HANDLER_ACTIVE" not in gov_src:
            miss.append("크래시 핸들러 재진입 가드")
        # ④ 좀비 하트비트
        if "fn reap_zombie_surfaces" not in gov_src or "reap_zombie_surfaces(&daemon" not in gov_src:
            miss.append("reap_zombie_surfaces watchdog 결선")
        if "ZOMBIE_MISS_THRESHOLD" not in gov_src:
            miss.append("좀비 3-miss 임계")
        if miss:
            self.add(cid, FAIL, "거버넌스 경화 미배선: %s" % ", ".join(miss))
        else:
            self.add(cid, PASS, "오염격리(ProcessHealth)·바이트상한(cap_response)·무음크래시"
                     "(surface_crashed+재진입가드)·좀비(reap_zombie_surfaces 3-miss) 4가닥 배선 확인")

    # ── C49 onboarding catalog + directive cascade (T4-3 + T3-2) ──
    # 결정론 검증(LLM 재추론 금지): ① T4-3 런타임 카탈로그가 실제 레지스트리(EditKind=cys_kinds.json
    # edit_kind 단일진실)에서 파생되는지(하드코딩이면 드리프트) ② 플레이스홀더 치환이 directive 주입
    # 경로(compose_directive)에 배선됐는지 ③ on-demand 단건 RPC(editor.action_info)·전체 카탈로그
    # RPC(editor.action_catalog)가 등록됐는지 ④ T3-2 캐스케이드 모듈이 존재하는지. 소스 grep + codegen
    # 집합 대조(C44/C48 동형) — 추가 인프라 0.
    def c49_onboarding_catalog(self):
        cid = "C49.onboarding-catalog"
        if self.skipped(cid):
            return
        crate_root = os.path.dirname(os.path.abspath(pack_dir().rstrip("/")))
        ac_rs = os.path.join(crate_root, "src", "action_catalog.rs")
        dc_rs = os.path.join(crate_root, "src", "directive_compose.rs")
        cys_rs = os.path.join(crate_root, "src", "bin", "cys.rs")
        hnd_rs = os.path.join(crate_root, "src", "bin", "cysd", "handlers.rs")
        miss = []
        for label, p in (("action_catalog.rs", ac_rs), ("directive_compose.rs", dc_rs),
                         ("cys.rs", cys_rs), ("handlers.rs", hnd_rs)):
            if not os.path.isfile(p):
                miss.append("%s 부재" % label)
        if miss:
            self.add(cid, WARN, "T4-3/T3-2 소스 부재 — 검사 보류: %s" % ", ".join(miss))
            return
        try:
            ac = open(ac_rs, encoding="utf-8").read()
            cys_src = open(cys_rs, encoding="utf-8").read()
            hnd = open(hnd_rs, encoding="utf-8").read()
        except OSError as e:
            self.add(cid, WARN, "소스 읽기 불가 — 보류: %s" % e)
            return
        # ② 치환이 directive 주입 경로에 배선
        if "substitute_catalog" not in cys_src:
            miss.append("compose_directive에 substitute_catalog 미배선")
        # ① 카탈로그가 레지스트리 파생(EditKind::ALL 순회) — 하드코딩 아님
        if "EditKind::ALL" not in ac:
            miss.append("action_catalog가 EditKind::ALL 레지스트리 파생 아님(하드코딩 의심)")
        # ③ on-demand 단건 + 전체 카탈로그 RPC 등록
        if '"editor.action_info"' not in hnd:
            miss.append("editor.action_info RPC 미등록")
        if '"editor.action_catalog"' not in hnd:
            miss.append("editor.action_catalog RPC 미등록")
        # ① 강화: 카탈로그 액션 집합 == cys_kinds.json edit_kind(build.rs 단일진실) — 드리프트0.
        gen = _find_codegen_kinds()
        if gen is not None:
            kinds = set(gen.get("edit_kind", []))
            # action_catalog.rs의 action_summary match arm이 모든 edit_kind 리터럴을 덮는지
            # 직접 확인할 순 없으나(serde 파생), no-wildcard match라 빌드가 누락을 차단한다.
            # 여기선 카탈로그 모듈이 EditKind를 import하는지로 단일진실 결속만 박제.
            if "use crate::edit_kinds::EditKind" not in ac:
                miss.append("action_catalog가 edit_kinds 단일진실에 미결속")
            if not kinds:
                miss.append("cys_kinds.json edit_kind 비어있음")
        if miss:
            self.add(cid, FAIL, "온보딩 카탈로그/캐스케이드 미배선: %s" % ", ".join(miss))
        else:
            tail = "" if gen is not None else " (cys_kinds.json 미발견 — 빌드 후 레지스트리 대조 재검 가능)"
            self.add(cid, PASS, "T4-3 카탈로그 레지스트리파생+치환배선+on-demand RPC · "
                     "T3-2 캐스케이드 모듈 확인%s" % tail)

    # ── C50 스킬 self-describing 설치 매니페스트 (T6-P4 — requires.bins + 플랫폼별 install) ──
    # 각 스킬 SKILL.md의 `cys:` 프론트매터가 self-describe한 requires.bins를 shutil.which로
    # 결손 탐지하고, 결손 시 install 매니페스트에서 플랫폼(darwin→brew / linux→apt) 설치 명령을
    # 선택해 보고한다. --fix는 dry-run으로 선택된 명령만 출력한다(실제 brew/apt 실행은 denylist
    # 환경변경 — cys feed push 승인 게이트 경유가 정책이므로 preflight는 자동 실행하지 않는다).
    # 스코프: `cys:` 프론트매터에 requires.bins가 있는 스킬만(현재 영상 도구 의존 소수) — 전 스킬
    # 순회 아님(대부분 순수 LLM). 멱등: 존재하는 bin은 무동작, 매니페스트 오타·구조결함은 fail-loud.
    def c50_skill_requirements(self):
        cid = "C50.skill-requirements"
        if self.skipped(cid):
            return
        skills_root = os.path.join(pack_dir(), "skills")
        if not os.path.isdir(skills_root):
            self.add(cid, WARN, "skills/ 부재 — 스킬 설치 매니페스트 검사 보류")
            return
        plat_kind, plat_field = ("brew", "formula") if sys.platform == "darwin" else ("apt", "package")
        checked = 0          # requires.bins 선언 스킬 수
        malformed = []       # 매니페스트 구조 결함(fail-loud)
        missing = []         # (skill, bin, install_cmd|None)
        for name in sorted(os.listdir(skills_root)):
            sp = os.path.join(skills_root, name, "SKILL.md")
            if not os.path.isfile(sp):
                continue
            try:
                text = open(sp, encoding="utf-8").read()
            except OSError:
                continue
            req_bins, install = _parse_skill_requires(text)
            if req_bins is None:
                continue  # requires.bins 미선언 — 스코프 밖(순수 LLM 스킬)
            checked += 1
            if not isinstance(req_bins, list) or not all(isinstance(b, str) for b in req_bins):
                malformed.append("%s: requires.bins가 문자열 배열 아님" % name)
                continue
            for b in req_bins:
                if shutil.which(b):
                    continue  # 존재 — 멱등 무동작
                # 결손: 플랫폼별 install 명령 선택
                cmd = None
                for entry in (install or []):
                    if entry.get("kind") == plat_kind and entry.get(plat_field):
                        cmd = "%s install %s" % (plat_kind, entry[plat_field])
                        break
                if cmd is None and install:
                    malformed.append("%s: bin '%s' 결손이나 %s install 매니페스트 없음/오타"
                                     % (name, b, plat_kind))
                missing.append((name, b, cmd))
        if malformed:
            self.add(cid, FAIL, "스킬 설치 매니페스트 결함(fail-loud): %s" % "; ".join(malformed))
            return
        if missing:
            lines = ["%s→%s: %s" % (s, b, cmd or "(이 플랫폼 install 명령 없음)")
                     for s, b, cmd in missing]
            if self.fix:
                self.add(cid, FIXED,
                         "결손 바이너리 %d건 — dry-run 설치 명령(실행 보류: cys feed push 승인 게이트): %s"
                         % (len(missing), "; ".join(lines)))
            else:
                self.add(cid, FAIL,
                         "스킬 요구 바이너리 결손 %d건 — `--fix`로 플랫폼별 설치 명령 확인: %s"
                         % (len(missing), "; ".join(lines)))
            return
        self.add(cid, PASS,
                 "스킬 self-describing 요구 바이너리 충족(requires.bins 선언 스킬 %d개·%s 플랫폼)"
                 % (checked, plat_kind))

    # ── C51 verdict 단일 머신-SOT (T6-P5 — verdict_schema.json 에서 런타임 로드 + 정합) ──
    # T1-1/C43 은 리터럴 *드리프트*를 잡았다. T6-P5 는 그 리터럴을 한 소스(verdict_schema.json)에서
    # *파생*시킨다. C51 은 결정론 검증한다: ① 스키마 파일 존재·유효 JSON ② javis_verdict.py 가
    # 실제로 스키마를 로드(SCHEMA_LOADED=True, 인라인 폴백이 아님) ③ 스키마 VERDICT_ENUM/SEVERITY/
    # ISSUE_CONTRACT 가 verdict.py 가 노출하는 값과 일치(스키마↔코드 정합) ④ INVESTIGATE 화해 —
    # reviewer_enum(4) + validator_emitted([INVESTIGATE]) 가 VERDICT_ENUM(5)을 분할하는지. 코드측
    # 무결성(self-test)은 C36 에 위임(중복 호출 회피) — 여기선 *스키마 결속*만 본다.
    def c51_verdict_schema_sot(self):
        cid = "C51.verdict-schema-sot"
        if self.skipped(cid):
            return
        schema_p = os.path.join(pack_dir(), "schemas", "verdict_schema.json")
        vbin = os.path.join(pack_dir(), "bin", "javis_verdict.py")
        if not os.path.isfile(vbin):
            self.add(cid, WARN, "javis_verdict.py 부재 — verdict 스키마 SOT 검사 보류")
            return
        if not os.path.isfile(schema_p):
            self.add(cid, FAIL, "verdict_schema.json 부재 — 단일 머신-SOT 미설치(T6-P5)")
            return
        try:
            s = json.load(open(schema_p, encoding="utf-8"))
        except (OSError, ValueError) as e:
            self.add(cid, FAIL, "verdict_schema.json 유효 JSON 아님: %s" % e)
            return
        # ② verdict.py 가 스키마를 실제 로드하는지 — subprocess 격리(import 부작용 회피, c36/c43 패턴).
        probe = (
            "import importlib.util,json,sys;"
            "spec=importlib.util.spec_from_file_location('v',%r);"
            "v=importlib.util.module_from_spec(spec);spec.loader.exec_module(v);"
            "print(json.dumps({'loaded':v.SCHEMA_LOADED,'VERDICT_ENUM':list(v.VERDICT_ENUM),"
            "'SEVERITY_ENUM':list(v.SEVERITY_ENUM),'ISSUE_CONTRACT':list(v.ISSUE_CONTRACT)}))"
            % vbin
        )
        r = subprocess.run([sys.executable, "-c", probe], capture_output=True, text=True)
        if r.returncode != 0:
            self.add(cid, FAIL, "javis_verdict.py 로드 실패 — %s" % (r.stderr.strip()[:160]))
            return
        try:
            got = json.loads(r.stdout.strip().splitlines()[-1])
        except (ValueError, IndexError):
            self.add(cid, FAIL, "verdict.py 스키마 프로브 출력 파싱 불가")
            return
        if not got.get("loaded"):
            self.add(cid, FAIL, "javis_verdict.py 가 verdict_schema.json 을 로드하지 않음 "
                     "(인라인 폴백 — 단일 SOT 미결속)")
            return
        # ③ 스키마↔코드 정합
        for key in ("VERDICT_ENUM", "SEVERITY_ENUM", "ISSUE_CONTRACT"):
            if set(s.get(key, [])) != set(got.get(key, [])):
                self.add(cid, FAIL, "%s 스키마↔코드 불일치 — 스키마:%s / 코드:%s"
                         % (key, sorted(s.get(key, [])), sorted(got.get(key, []))))
                return
        # ④ INVESTIGATE 화해: reviewer_enum(4) + validator_emitted == VERDICT_ENUM(5)
        rev = set(s.get("reviewer_enum", []))
        emit = set(s.get("validator_emitted", []))
        full = set(s.get("VERDICT_ENUM", []))
        if rev | emit != full or rev & emit:
            self.add(cid, FAIL, "INVESTIGATE 화해 깨짐 — reviewer_enum∪validator_emitted ≠ "
                     "VERDICT_ENUM (rev:%s emit:%s full:%s)" % (sorted(rev), sorted(emit), sorted(full)))
            return
        if "INVESTIGATE" not in emit or "INVESTIGATE" in rev:
            self.add(cid, FAIL, "INVESTIGATE 는 validator-only 여야 함 — reviewer_enum 에 누출 또는 "
                     "validator_emitted 누락")
            return
        self.add(cid, PASS,
                 "verdict 단일 머신-SOT 결속 — verdict_schema.json 로드됨·스키마↔코드 정합 "
                 "(VERDICT/SEVERITY/ISSUE_CONTRACT)·INVESTIGATE=validator-only 화해 OK")

    # ── C09 round 핵심 문서 ──
    def c09_round_core(self):
        cid = "C09.round-core"
        if self.skipped(cid):
            return
        missing = []
        for f in ("SESSION_STATE.md", "RECOVERY.md"):
            p = os.path.join(pack_dir(), "round", f)
            if not os.path.isfile(p):
                missing.append(f)
        if missing and self.fix and self.repair_via_init_pack():
            missing = [
                f for f in missing
                if not os.path.isfile(os.path.join(pack_dir(), "round", f))
            ]
            if not missing:
                self.add(cid, FIXED, "round 핵심 문서 재설치")
                return
        if missing:
            self.add(cid, FAIL, "누락: %s" % ", ".join(missing))
        else:
            self.add(cid, PASS, "SESSION_STATE.md·RECOVERY.md 존재")

    # ── C10 전 노드 TODO 영속 파일 (절대지침 7) ──
    def c10_todo_files(self):
        cid = "C10.todo-files"
        if self.skipped(cid):
            return
        rdir = os.path.join(pack_dir(), "round")
        missing = [f for f in TODO_FILES if not os.path.isfile(os.path.join(rdir, f))]
        if not missing:
            self.add(cid, PASS, "4개 노드 TODO 전부 존재")
            return
        if self.fix:
            os.makedirs(rdir, exist_ok=True)
            for f in missing:
                node = f.replace("_TODO.md", "")
                open(os.path.join(rdir, f), "w", encoding="utf-8").write(
                    "# %s_TODO — 영속 todo (절대지침 7)\n\n"
                    "> 세부 완료마다 갱신·디스크 영속. 세션 clear/재시작 후 이 파일부터 읽고 복원한다.\n\n"
                    "- [ ] (작업을 추가하라)\n" % node
                )
            self.add(cid, FIXED, "생성: %s" % ", ".join(missing))
        else:
            self.add(cid, FAIL, "누락: %s — --fix로 생성 가능" % ", ".join(missing))

    # ── C11 cys 바이너리 ──
    def c11_cys_binary(self):
        cid = "C11.cys-binary"
        if self.skipped(cid):
            return
        p = shutil.which("cys")
        if p:
            self.add(cid, PASS, p)
        else:
            self.add(cid, FAIL, "PATH에 cys 없음 — cys 터미널 설치/PATH 확인 필요")

    # ── C12 cysd 데몬 생존 ──
    def c12_daemon(self):
        cid = "C12.daemon"
        if self.skipped(cid):
            return
        cys = shutil.which("cys")
        if not cys:
            self.add(cid, SKIP, "cys 부재로 판정 불가 (C11 먼저)")
            return

        def ping():
            try:
                return subprocess.run(
                    [cys, "ping"], capture_output=True, timeout=5
                ).returncode == 0
            except Exception:
                return False

        if ping():
            self.add(cid, PASS, "cys ping OK")
            return
        cysd = shutil.which("cysd")
        if self.fix and cysd:
            log = open("/tmp/cysd-preflight.log", "ab") if os.name == "posix" else subprocess.DEVNULL
            subprocess.Popen(
                [cysd], stdout=log, stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            for _ in range(10):
                time.sleep(0.5)
                if ping():
                    self.add(cid, FIXED, "cysd 기동 후 ping OK")
                    return
            self.add(cid, FAIL, "cysd 기동 시도했으나 ping 실패 — /tmp/cysd-preflight.log 확인")
        else:
            self.add(cid, FAIL, "데몬 다운 — `cysd > /tmp/cysd.log 2>&1 &` 후 재실행 (--fix로 자동 기동 가능)")

    # ── C13 프로젝트 CLAUDE.md (git 루트에서만) ──
    def c13_claude_md(self):
        cid = "C13.claude-md"
        if self.skipped(cid):
            return
        if not os.path.isdir(".git"):
            self.add(cid, SKIP, "cwd가 git 루트 아님")
            return
        if os.path.isfile("CLAUDE.md"):
            self.add(cid, PASS, "프로젝트 CLAUDE.md 존재")
            return
        tpl = os.path.join(pack_dir(), "CLAUDE.md.template")
        if self.fix and os.path.isfile(tpl):
            shutil.copy2(tpl, "CLAUDE.md")
            self.add(cid, FIXED, "CLAUDE.md.template → ./CLAUDE.md 배치")
        else:
            self.add(cid, WARN, "프로젝트 CLAUDE.md 없음 (hook이 전역 커버하므로 권장 수준) — --fix로 배치 가능")

    # ── C14 프리플라이트 자기 존재 (pack 영구 편입 확인) ──
    def c14_self(self):
        cid = "C14.preflight-self"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "bin", "javis_preflight.py")
        if os.path.isfile(p):
            self.add(cid, PASS, p)
            return
        if self.fix:
            os.makedirs(os.path.dirname(p), exist_ok=True)
            shutil.copy2(os.path.abspath(__file__), p)
            os.chmod(p, 0o755)
            self.add(cid, FIXED, "자기 복제로 pack에 편입: %s" % p)
        else:
            self.add(cid, FAIL, "pack/bin/javis_preflight.py 없음 — `cys init-pack` 또는 --fix")

    # ── C15 진행% 보고기 javis_report.py (앵커3-A6) ──
    def c15_report_tool(self):
        cid = "C15.report-tool"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "bin", "javis_report.py")
        if not os.path.isfile(p):
            if self.fix and self.repair_via_init_pack() and os.path.isfile(p):
                self.add(cid, FIXED, "javis_report.py 재설치")
            else:
                self.add(cid, FAIL, "pack/bin/javis_report.py 없음 — `cys init-pack` 또는 --fix")
            return
        self.add(cid, PASS, p)

    # ── C16 5분 주기 보고 스케줄 job (앵커3-A6) ──
    def c16_report_schedule(self):
        cid = "C16.report-schedule"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "schedule.json")
        try:
            data = json.load(open(p, encoding="utf-8"))
        except (OSError, ValueError) as e:
            self.add(cid, FAIL, "schedule.json 읽기/파싱 실패: %s (C06 먼저)" % e)
            return
        jobs = data.get("jobs", [])
        # 절대지침 "매 5분" — every_minutes는 5 이하만 충족(더 자주는 명세 이상, 더 길면 위반).
        # 결정론 환원: text_command(데몬이 javis_report 실행)가 권장이나, text도 허용한다.
        def is_report(j):
            return (isinstance(j.get("every_minutes"), int)
                    and j.get("action") == "push" and j.get("to") == "master"
                    and (j.get("text") or j.get("text_command")))
        rep = [j for j in jobs if is_report(j) and 1 <= j.get("every_minutes") <= 5]
        too_slow = [j for j in jobs if is_report(j) and j.get("every_minutes") > 5]
        if rep:
            j = rep[0]
            mode = "text_command(결정론 직접산출)" if j.get("text_command") else "text(master 산출)"
            self.add(cid, PASS, "5분 보고 job 존재: %s (every_minutes=%s ≤5, %s)"
                     % (j.get("id"), j.get("every_minutes"), mode))
            return
        if too_slow and not self.fix:
            j = too_slow[0]
            self.add(cid, FAIL, "보고 주기가 너무 김: %s (every_minutes=%s > 5) — 절대지침 5분 위반"
                     % (j.get("id"), j.get("every_minutes")))
            return
        if self.fix:
            jobs.append({
                "id": "owner-progress-report-5min",
                "every_minutes": 5,
                "action": "push",
                "to": "master",
                "text_command": ('printf \'[heartbeat] 5분 보고 — 아래 진행%%는 결정론 산출값이다. '
                                 '그대로(수치 불변) 주인님에게 보고하라.\\n\'; '
                                 'python3 "${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_report.py"'),
                "if_absent": "skip",
            })
            data["jobs"] = jobs
            open(p, "w", encoding="utf-8").write(
                json.dumps(data, ensure_ascii=False, indent=2))
            self.add(cid, FIXED, "5분 보고 job(owner-progress-report-5min) 추가")
        else:
            self.add(cid, FAIL, "5분 주기 master 보고 job 부재 — --fix로 추가 가능")

    # ── 공용: pack/bin 도구 존재 확보 + --self-test 실행 ──
    def _check_bin_tool(self, cid, fname, extra_files=()):
        """bin 도구의 존재(누락 시 init-pack 수리)·자기검증을 결정론으로 판정한다."""
        p = os.path.join(pack_dir(), "bin", fname)
        missing = [f for f in (fname,) + tuple(extra_files)
                   if not os.path.isfile(os.path.join(pack_dir(), "bin", f))]
        if missing and self.fix and self.repair_via_init_pack():
            missing = [f for f in missing
                       if not os.path.isfile(os.path.join(pack_dir(), "bin", f))]
        if missing:
            self.add(cid, FAIL, "pack/bin 누락: %s — `cys init-pack` 또는 --fix"
                     % ", ".join(missing))
            return None
        if os.name == "posix" and not os.stat(p).st_mode & stat.S_IXUSR and self.fix:
            os.chmod(p, 0o755)
        try:
            r = subprocess.run([sys.executable, p, "--self-test"],
                               capture_output=True, timeout=30)
        except Exception as e:
            self.add(cid, FAIL, "%s --self-test 실행 불가: %s" % (fname, e))
            return None
        if r.returncode != 0:
            tail = (r.stdout or r.stderr or b"").decode("utf-8", "replace").strip()
            self.add(cid, FAIL, "%s --self-test 실패: %s" % (fname, tail[-400:]))
            return None
        return p

    # ── C17 3단 사고 라우팅 결정론 엔진 (사고 모드 §1) ──
    def c17_route_engine(self):
        cid = "C17.route-engine"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_route.py",
                                 extra_files=("route_triggers.json",))
        if p:
            self.add(cid, PASS, "%s self-test OK (로직 배터리 + 트리거 구조 검증)" % p)

    # ── C19 LLM 오케스트레이션 결정론 도구 (앵커4) ──
    def c19_orchestra_engine(self):
        cid = "C19.orchestra-engine"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_orchestra.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (4종 노드·라운드·제약 주입 검증)" % p)

    # ── C34 자기기술 능력 레지스트리 (OpenMontage D2 — 하드코딩 목록 폐기·파생 카탈로그) ──
    def c34_registry(self):
        cid = "C34.registry"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_registry.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (능력 카탈로그 파생·orphan lint·무점수)" % p)

    # ── C35 채점식 provider 선택 엔진 (OpenMontage P5 — deny-by-default·무료우선) ──
    def c35_select(self):
        cid = "C35.select"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_select.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (7차원 채점·deny-by-default·무료우선)" % p)

    # ── C36 리뷰어 verdict 스키마검증 + CHAI lint (OpenMontage D1 — 4자수렴 기계검증부) ──
    def c36_verdict(self):
        cid = "C36.verdict"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_verdict.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (verdict 계약·점수금지·CHAI R2 강등)" % p)

    # ── C37 의사결정 로그(OpenMontage D3 — Options Considered·rejected_because·근거·무점수) ──
    def c37_adr_engine(self):
        cid = "C37.adr-engine"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_adr.py")
        if p:
            self.add(cid, PASS, "%s self-test OK (결정근거 rationale·커버리지 게이트·동거 ledger)" % p)

    # ── C38 무음실패 카탈로그 (OpenMontage D5 2부 — 런타임-write 미러[C10/C16/C25]·드리프트 WARN·--fix 재생성) ──
    # _check_bin_tool 아님: 검사 대상은 bin 도구가 아니라 round/ 런타임 아티팩트다. SILENT_FAILURES
    # 단일 source-of-record를 보존하려 렌더/드리프트 판정은 javis_orchestra에 위임(subprocess).
    def c38_silent_failure_catalog(self):
        cid = "C38.silent-failure-catalog"
        if self.skipped(cid):
            return
        orch = os.path.join(pack_dir(), "bin", "javis_orchestra.py")
        cat = os.path.join(pack_dir(), "round", "SILENT_FAILURE_CATALOG.md")
        if not os.path.isfile(orch):
            self.add(cid, WARN, "javis_orchestra.py 부재 — 무음실패 카탈로그 검사 보류")
            return
        try:
            if self.fix:
                r = subprocess.run([sys.executable, orch, "silent-failure-catalog"],
                                   capture_output=True, timeout=30)
                if r.returncode == 0:
                    self.add(cid, FIXED, "무음실패 카탈로그 재생성: %s" % cat)
                else:
                    tail = (r.stderr or r.stdout or b"").decode("utf-8", "replace").strip()
                    self.add(cid, WARN, "무음실패 카탈로그 재생성 실패: %s" % tail[-200:])
                return
            r = subprocess.run([sys.executable, orch, "silent-failure-catalog", "--check"],
                               capture_output=True, timeout=30)
            if r.returncode == 0:
                self.add(cid, PASS, "무음실패 카탈로그 정합 (런타임 파생·D5 거버넌스)")
            else:
                self.add(cid, WARN, "무음실패 카탈로그 드리프트/부재 — `javis_preflight.py --fix` 또는 "
                         "`javis_orchestra.py silent-failure-catalog`로 재생성")
        except Exception as e:
            # WARN-only 계약 유지: 카탈로그 검사 실패(타임아웃·OSError)가 전체 preflight를 죽이지
            # 않게 한다(형제 검사 C18·_check_bin_tool의 except Exception 패턴 미러).
            self.add(cid, WARN, "무음실패 카탈로그 검사 실행 불가 — 보류: %s" % e)

    # ── C39 전제지식 고아 lint (OpenMontage D6 — requires_skills/related_memory 슬러그 해소·WARN-only) ──
    # registry verify에 위임(normalize_slug·색인 규칙 단일 source-of-record 보존 — preflight는
    # orchestra/registry를 import 안 함). orphan 문제만 골라 WARN(드리프트·점수 위반은 registry 몫).
    # orphan 탐지 *로직* 정합은 C34(registry --self-test, synthetic orphan-ref/mem)가 핀하고,
    # C39는 그 로직을 라이브 pack 데이터에 적용하는 표면이다(C19↔orchestra 쌍과 동형).
    def c39_prereq_orphan_lint(self):
        cid = "C39.prereq-orphan-lint"
        if self.skipped(cid):
            return
        reg = os.path.join(pack_dir(), "bin", "javis_registry.py")
        if not os.path.isfile(reg):
            self.add(cid, WARN, "javis_registry.py 부재 — 전제지식 고아 lint 보류")
            return
        try:
            # --root로 lint 대상을 preflight가 보는 pack에 핀(env 재유도 분기 차단).
            r = subprocess.run([sys.executable, reg, "verify", "--root", pack_dir(), "--json"],
                               capture_output=True, timeout=30)
            data = json.loads((r.stdout or b"").decode("utf-8", "replace") or "{}")
        except Exception as e:
            self.add(cid, WARN, "전제지식 고아 lint 실행 불가 — 보류: %s" % e)
            return
        orphans = [p for p in data.get("problems", []) if p.startswith("orphan ")]
        if orphans:
            self.add(cid, WARN, "전제지식 고아 %d건(requires_skills/related_memory 색인 미해소) — %s"
                     % (len(orphans), " · ".join(orphans[:3])))
        else:
            self.add(cid, PASS, "전제지식 고아 0 — requires_skills/related_memory 색인 해소 정합")

    # ── C40 워크플로우 매니페스트 도구 (OpenMontage D4 — 신규 *옵션* 도구·WARN-only) ──
    # _check_bin_tool 아님: 그건 부재·self-test 실패를 FAIL로 만든다. 매니페스트는 opt-in
    # (없으면 resolve exit 4 → README 디스패치 폴백)이라 boot-blocker가 아니다 → WARN.
    def c40_workflow_manifest(self):
        cid = "C40.workflow-manifest"
        if self.skipped(cid):
            return
        p = os.path.join(pack_dir(), "bin", "javis_manifest.py")
        if not os.path.isfile(p):
            self.add(cid, WARN, "javis_manifest.py 부재 — 타입드 워크플로우 매니페스트 미설치(opt-in·README 폴백)")
            return
        try:
            r = subprocess.run([sys.executable, p, "--self-test"],
                               capture_output=True, timeout=30)
        except Exception as e:
            self.add(cid, WARN, "javis_manifest.py --self-test 실행 불가 — 보류: %s" % e)
            return
        if r.returncode == 0:
            self.add(cid, PASS, "javis_manifest.py self-test OK (매니페스트 계약·무점수·콘텐츠 checks·exit 4 폴백)")
        else:
            tail = (r.stdout or r.stderr or b"").decode("utf-8", "replace").strip()
            self.add(cid, WARN, "javis_manifest.py self-test 실패(도구 점검 필요) — %s" % tail[-200:])

    # ── C18 장기기억 증류 결정론 도구 + 색인↔파일 정합 (§10 증류 게이트) ──
    def c18_memory_engine(self):
        cid = "C18.memory-engine"
        if self.skipped(cid):
            return
        p = self._check_bin_tool(cid, "javis_memory.py")
        if not p:
            return
        # 실 데이터 정합 — MEMORY.md 색인과 메모리 파일의 기계검증.
        # 자동 수리 없음: 기억 내용은 오너·노드 소관이라 preflight가 임의 재작성하지 않는다.
        try:
            r = subprocess.run([sys.executable, p, "verify", "--json"],
                               capture_output=True, timeout=15)
        except Exception as e:
            self.add(cid, FAIL, "javis_memory verify 실행 불가: %s" % e)
            return
        if r.returncode == 0:
            self.add(cid, PASS, "self-test OK + 장기기억 색인↔파일 정합")
        else:
            tail = (r.stdout or b"").decode("utf-8", "replace").strip()
            self.add(cid, FAIL, "장기기억 부정합 — 수동 복구 필요: %s" % tail[-400:])

    # ── C20 보조: nlm 버전 탐지 / 설치 / MCP 등록 ──
    @staticmethod
    def _nlm_version():
        nlm = shutil.which("nlm")
        if not nlm:
            return None, None
        try:
            out = subprocess.run([nlm, "--version"], capture_output=True,
                                 timeout=15).stdout.decode("utf-8", "replace")
            m = re.search(r"(\d+)\.(\d+)\.(\d+)", out)
            return nlm, (tuple(int(x) for x in m.groups()) if m else None)
        except Exception:
            return nlm, None

    @staticmethod
    def _install_nlm():
        """uv → pipx → pip 폴백으로 핀 버전 설치. 성공 여부 반환."""
        candidates = []
        if shutil.which("uv"):
            candidates.append(["uv", "tool", "install", "--force", NLM_PIN])
        if shutil.which("pipx"):
            candidates.append(["pipx", "install", "--force", NLM_PIN])
        candidates.append([sys.executable, "-m", "pip", "install", "--user",
                           "--upgrade", NLM_PIN])
        for cmd in candidates:
            try:
                if subprocess.run(cmd, capture_output=True, timeout=600).returncode == 0:
                    return True
            except Exception:
                continue
        return False

    def _register_mcp(self, mcp_path, name, binary, env=None):
        """프로젝트 .mcp.json에 MCP 서버 등록(merge). 성공=None, 실패=사유.
        binary는 PATH에서 절대경로로 해석해 박는다. env는 그대로 기입
        (값에 ${VAR}를 쓰면 Claude Code가 세션 환경변수로 전개한다)."""
        if os.path.islink(mcp_path):
            return "symlink 거부: %s" % mcp_path
        server = shutil.which(binary)
        if not server:
            return "%s 실행파일 미발견 (설치 먼저)" % binary
        data = {}
        if os.path.isfile(mcp_path):
            try:
                data = json.load(open(mcp_path, encoding="utf-8"))
            except (OSError, ValueError) as e:
                return "기존 .mcp.json 파싱 실패 — 덮어쓰기 거부: %s" % e
            if not isinstance(data, dict):
                return ".mcp.json 루트가 객체가 아님 — 거부"
            backup = mcp_path + ".bak-preflight"
            if not os.path.exists(backup):
                shutil.copy2(mcp_path, backup)
        entry = {"command": server}
        if env:
            entry["env"] = env
        data.setdefault("mcpServers", {})[name] = entry
        # 원자적 쓰기 — settings.json 쓰기와 동일 사유(파손 시 수리 불능 차단).
        tmp = mcp_path + ".tmp"
        open(tmp, "w", encoding="utf-8").write(
            json.dumps(data, ensure_ascii=False, indent=2))
        os.replace(tmp, mcp_path)
        return None

    @staticmethod
    def _mcp_registered(mcp_path, name):
        """mcpServers에 정확한 서버 키가 있는가 — 전체 JSON 부분문자열 검사는
        무관한 값(경로·URL)에 오탐해 --fix가 실제 등록을 영영 건너뛴다."""
        if not os.path.isfile(mcp_path):
            return False
        try:
            cfg = json.load(open(mcp_path, encoding="utf-8"))
        except (OSError, ValueError):
            return False
        return isinstance(cfg, dict) and name in cfg.get("mcpServers", {})

    def _register_nlm_mcp(self, mcp_path):
        return self._register_mcp(mcp_path, "notebooklm-mcp", "notebooklm-mcp")

    # ── C20 NotebookLM SOT 도구 (nlm CLI + MCP 등록 + 인증) ──
    # 자동화 경계(오너 확정 2026-06-12): 설치·MCP 등록은 기계가 수행(--fix),
    # Google 로그인은 사람 전용 단계 — "빠진 것을 기계가 알려주는" 수준으로
    # 정확한 명령을 안내한다(부트 비차단 WARN).
    def c20_nlm_sot(self):
        cid = "C20.nlm-sot"
        if self.skipped(cid):
            return
        nlm, ver = self._nlm_version()
        fixed = []
        # (a) 설치·버전 하한
        if nlm is None or ver is None or ver < NLM_MIN_VERSION:
            cur = ".".join(map(str, ver)) if ver else "미설치/판독불가"
            if self.fix and self._install_nlm():
                nlm, ver = self._nlm_version()
            if nlm and ver and ver >= NLM_MIN_VERSION:
                fixed.append("nlm %s 설치(핀)" % ".".join(map(str, ver)))
            else:
                self.add(cid, FAIL,
                         "nlm %s — SOT 도구 미비. --fix(uv/pipx/pip 자동 설치) 또는 "
                         "`uv tool install '%s'`" % (cur, NLM_PIN))
                return
        # (b) MCP 등록 (git 루트에서만 — C13과 동일 스코프. worktree는 .git이 파일)
        mcp_note = ""
        mcp_err = False
        if os.path.exists(".git"):
            registered = self._mcp_registered(".mcp.json", "notebooklm-mcp")
            if not registered:
                if self.fix:
                    err = self._register_nlm_mcp(".mcp.json")
                    if err:
                        mcp_note = " · MCP 등록 실패: %s" % err
                        mcp_err = True
                    else:
                        fixed.append("./.mcp.json에 notebooklm-mcp 등록")
                else:
                    mcp_note = " · ./.mcp.json MCP 미등록(--fix로 등록 가능)"
        # (c) 인증 — 사람 전용 단계: 기계는 상태와 다음 명령만 정확히 알린다
        auth_ok = False
        try:
            auth_ok = subprocess.run([nlm, "login", "--check"], capture_output=True,
                                     timeout=45).returncode == 0
        except Exception:
            pass
        ver_s = ".".join(map(str, ver))
        suffix = (" · " + "; ".join(fixed)) if fixed else ""
        if not auth_ok:
            self.add(cid, WARN,
                     "nlm %s 설치됨%s · Google 미인증 — 사람 단계: `nlm login` 실행 필요%s"
                     % (ver_s, mcp_note, suffix))
            return
        if mcp_err:
            # 등록 실패를 PASS 본문에 접어 넣으면 READY가 MCP 계층 파손을 가린다.
            self.add(cid, WARN, "nlm %s · 인증 OK%s%s" % (ver_s, mcp_note, suffix))
            return
        self.add(cid, FIXED if fixed else PASS,
                 "nlm %s · 인증 OK%s%s" % (ver_s, mcp_note, suffix))

    # ── C21 Harness Creator 툴체인 (오너 제작 메타스킬의 도구 본체) ──
    # 스킬은 pack 임베드로 자동 배포 — 이 검사는 스킬이 호출하는 TOOLS_ROOT의 존재를
    # 결정론 검증하고, 신규 머신에서는 --fix가 핀 커밋을 자동 클론한다.
    @staticmethod
    def _harness_root():
        cands = []
        env = os.environ.get("CYS_HARNESS_HOME", "")
        if env:
            cands.append(env)
        home = os.path.expanduser("~")
        cands.append(os.path.join(home, ".cys/harness-creator"))
        cands.append(os.path.join(home, "Desktop/CYSjavis/cys-harness-creator"))
        for d in cands:
            if all(os.path.isfile(os.path.join(d, f)) for f in HARNESS_KEY_FILES):
                return d
        return None

    def c21_harness_creator(self):
        cid = "C21.harness-creator"
        if self.skipped(cid):
            return
        root = self._harness_root()
        if root:
            self.add(cid, PASS, "TOOLS_ROOT=%s (핵심 도구 %d종 존재)"
                     % (root, len(HARNESS_KEY_FILES)))
            return
        dst = os.path.join(os.path.expanduser("~"), ".cys/harness-creator")
        if self.fix and shutil.which("git"):
            try:
                ok = subprocess.run(["git", "clone", HARNESS_REPO, dst],
                                    capture_output=True, timeout=300).returncode == 0
                if ok:
                    # 핀은 검증돼야 핀이다 — checkout rc와 HEAD==핀을 기계 확인하지
                    # 않으면 핀 부재(force-push·레포 교체) 시 조용히 moving HEAD로
                    # 남아 FIXED가 거짓 핀 주장이 된다(공급망 표면).
                    co = subprocess.run(["git", "-C", dst, "checkout", HARNESS_PIN],
                                        capture_output=True, timeout=60).returncode
                    head = subprocess.run(
                        ["git", "-C", dst, "rev-parse", "HEAD"],
                        capture_output=True, timeout=15).stdout.decode().strip()
                    ok = co == 0 and head == HARNESS_PIN
            except Exception:
                ok = False
            if ok and self._harness_root():
                self.add(cid, FIXED, "%s 클론(핀 %s 검증)" % (dst, HARNESS_PIN[:8]))
                return
        dirty = " (기존 %s 불완전 — 제거 후 재시도 필요)" % dst if os.path.isdir(dst) else ""
        self.add(cid, FAIL,
                 "harness-creator 툴체인 미설치%s — --fix(git 자동 클론) 또는 "
                 "`git clone %s %s && git -C %s checkout %s`"
                 % (dirty, HARNESS_REPO, dst, dst, HARNESS_PIN[:8]))

    # ── C22 work management 스킬 2종 (앵커5-4b·c — 환각방지·의도 합의) ──
    # 절대 강조 4규칙의 b(hallucination-guard)·c(grill-me)가 가리키는 전담 sub-skill이
    # 실재해야 지침이 공수표가 되지 않는다. 누락 시 init-pack 임베드로 수리한다.
    def _skill_indexable(self, name):
        p = os.path.join(pack_dir(), "skills", name, "SKILL.md")
        if not (os.path.isfile(p) and os.path.getsize(p) > 0):
            return False
        # 실파서(cys.rs compose_directive)는 read_to_string이라 전 파일 UTF-8 유효 +
        # name: 값 비어있지 않음을 요구한다 — 동일 규칙로 판정(거짓 PASS 차단).
        # 줄 분리도 rust str::lines와 동일하게 \n 기준(bare-CR 파일 parity — splitlines 금지).
        try:
            head = open(p, encoding="utf-8", newline="").read().split("\n")[:10]
        except (OSError, UnicodeDecodeError):
            return False
        # rust는 첫 10줄에서 마지막 name: 이 이긴다(덮어쓰기 루프) — first-match로
        # 판정하면 'name: foo' 뒤 빈 'name:'이 있는 파일을 rust는 떨구는데 여기는
        # 통과시키는 거짓 PASS가 난다(parity 위반).
        name = None
        for ln in head:
            if ln.startswith("name:"):
                name = ln[5:].strip()
        return bool(name)

    def _work_skill_problem(self, name):
        """None=건전, 문자열=결함 사유. 색인성 + 본문 핀(전담 기능 실재)을 함께 판정."""
        if not self._skill_indexable(name):
            return "%s(누락/색인 불가)" % name
        p = os.path.join(pack_dir(), "skills", name, "SKILL.md")
        try:
            text = open(p, encoding="utf-8").read()
        except (OSError, UnicodeDecodeError):
            return "%s(읽기 실패)" % name
        lost = [pin for pin in WORK_SKILL_PINS.get(name, []) if pin not in text]
        if lost:
            return "%s(본문 핀 소실: %s)" % (name, "·".join(lost))
        return None

    def c22_work_skills(self):
        cid = "C22.work-skills"
        if self.skipped(cid):
            return
        problems = [pr for s in WORK_SKILLS if (pr := self._work_skill_problem(s))]
        repaired = []
        if problems and self.fix and self.repair_via_init_pack():
            still = [pr for s in WORK_SKILLS if (pr := self._work_skill_problem(s))]
            repaired = [pr for pr in problems if pr not in still]
            problems = still
        if problems:
            self.add(cid, FAIL,
                     "work 스킬 결함: %s — `cys init-pack` 또는 --fix"
                     "(파일이 존재하되 깨진/약화된 경우 init-pack은 보존한다 — "
                     "`cys init-pack --force`로 템플릿 복원, 사용자 수정 덮어씀 주의)"
                     % "; ".join(problems))
        elif repaired:
            self.add(cid, FIXED, "init-pack 수리 완료: %s" % "; ".join(repaired))
        else:
            self.add(cid, PASS,
                     "work management 스킬 2종(%s) 존재·색인 가능·본문 핀 건재"
                     % ", ".join(WORK_SKILLS))

    # ── C23 거버넌스 충돌 감시 (외부 에이전트 운영체계 동거 감지) ──
    # 사용자가 나중에 gstack류를 추가 설치해도 "아무도 모르는" 상황을 차단한다 —
    # 부트마다 결정론 감지 → WARN + 격리 수칙 안내 (금지·자동 제거 없음: 설치는 오너 주권).
    def c23_governance_conflict(self):
        cid = "C23.governance-conflict"
        if self.skipped(cid):
            return
        findings = []
        for settings_path in discover_claude_settings():
            profile = os.path.dirname(settings_path)
            # 충돌 조건 = cysjavis 배선 프로필(우리 hook 등록)과의 '동거'만
            if not self._hook_registered(settings_path):
                continue
            for name, sig in FOREIGN_AGENT_OS.items():
                signals = []
                if os.path.isdir(os.path.join(profile, "skills", sig["skills_dir"])):
                    signals.append("skills/%s 설치" % sig["skills_dir"])
                cmd_path = os.path.join(profile, "CLAUDE.md")
                if os.path.isfile(cmd_path):
                    try:
                        text = open(cmd_path, encoding="utf-8", errors="replace").read()
                        hits = [m for m in sig["claude_md_markers"] if m in text]
                        if hits:
                            signals.append("CLAUDE.md 점유 마커(%s)" % ", ".join(hits[:2]))
                    except OSError:
                        pass
                try:
                    stext = open(settings_path, encoding="utf-8", errors="replace").read()
                    if sig["hook_marker"] in stext:
                        signals.append("hook 등록")
                except OSError:
                    pass
                if signals:
                    findings.append("%s@%s: %s — %s"
                                    % (name, profile, "; ".join(signals), sig["guide"]))
        if findings:
            self.add(cid, WARN, "외부 운영체계 동거 감지 — " + " | ".join(findings))
        else:
            self.add(cid, PASS, "cysjavis 배선 프로필에 외부 운영체계 점유 신호 없음")

    # ── C24 한국 법령 전용 MCP (korean-law-mcp — k-skill law 프록시 경로 대체) ──
    # 자동화 경계는 C20과 동일: 설치·MCP 등록은 기계(--fix), 법제처 OC 키 발급만
    # 사람 단계로 정확히 안내한다(부트 비차단 WARN).
    @staticmethod
    def _klaw_version():
        """(cli경로|None, 버전튜플|None) — 설치·재설치 후 동일 경로로 재탐침한다."""
        cli = shutil.which("korean-law")
        if cli is None:
            return None, None
        try:
            out = subprocess.run([cli, "--version"], capture_output=True,
                                 timeout=15).stdout.decode("utf-8", "replace")
            m = re.search(r"(\d+)\.(\d+)\.(\d+)", out)
            return cli, (tuple(int(x) for x in m.groups()) if m else None)
        except Exception:
            return cli, None

    def c24_korean_law_mcp(self):
        cid = "C24.korean-law-mcp"
        if self.skipped(cid):
            return
        fixed = []
        cli, ver = self._klaw_version()
        # 버전 게이트는 C20과 동형으로 빈틈없이(else-망라) — ver 판독불가가
        # FAIL 없이 통과하던 무성 폴스루를 차단하고, 설치 후 버전을 재탐침한다.
        if cli is None or ver is None or ver < KLAW_MIN_VERSION:
            cur = ".".join(map(str, ver)) if ver else "미설치/판독불가"
            if self.fix and shutil.which("npm"):
                try:
                    if subprocess.run(["npm", "install", "-g", KLAW_PIN],
                                      capture_output=True, timeout=600).returncode == 0:
                        cli, ver = self._klaw_version()
                except Exception:
                    pass
            if cli and ver and ver >= KLAW_MIN_VERSION:
                fixed.append("%s 설치(핀)" % KLAW_PIN)
            else:
                self.add(cid, FAIL,
                         "korean-law %s — 법령 MCP 미비. --fix(npm 자동 설치) 또는 "
                         "`npm install -g %s`" % (cur, KLAW_PIN))
                return
        # 키 — 사람 전용 단계 (등록 전에 판정: 발견된 변수명을 등록에 그대로 쓴다)
        key_var = next((v for v in ("LAW_OC", "LAW_OC_ID") if os.environ.get(v)), None)
        # MCP 등록 (git 루트 — C20과 동일 스코프·worktree는 .git이 파일.
        #  ${변수}는 Claude Code가 세션 env에서 전개)
        mcp_note = ""
        mcp_err = False
        if os.path.exists(".git"):
            if not self._mcp_registered(".mcp.json", "korean-law-mcp"):
                if self.fix:
                    err = self._register_mcp(
                        ".mcp.json", "korean-law-mcp", "korean-law-mcp",
                        env={"LAW_OC": "${%s}" % (key_var or "LAW_OC")})
                    if err:
                        mcp_note = " · MCP 등록 실패: %s" % err
                        mcp_err = True
                    else:
                        fixed.append("./.mcp.json에 korean-law-mcp 등록")
                else:
                    mcp_note = " · ./.mcp.json MCP 미등록(--fix로 등록 가능)"
        suffix = (" · " + "; ".join(fixed)) if fixed else ""
        if not key_var:
            hint = "사람 단계: open.law.go.kr 가입·OC 발급 후 `export LAW_OC=<키>`"
            for rc in ("~/.zshrc", "~/.zshenv"):
                p = os.path.expanduser(rc)
                try:
                    if os.path.isfile(p) and "LAW_OC" in open(p, encoding="utf-8",
                                                              errors="replace").read():
                        hint = "%s에 키 라인 존재 — 현 프로세스 미로드(셸 재기동 필요)" % rc
                        break
                except OSError:
                    pass
            self.add(cid, WARN, "korean-law 설치됨%s · OC 키 미설정 — %s%s"
                     % (mcp_note, hint, suffix))
            return
        if mcp_err:
            self.add(cid, WARN, "korean-law-mcp · OC 키 확인%s%s" % (mcp_note, suffix))
            return
        self.add(cid, FIXED if fixed else PASS,
                 "korean-law-mcp · OC 키 확인%s%s" % (mcp_note, suffix))

    # ── C25 자율주행 메모리 상주 (앵커6 — 🔒색인 상주 필수) ──
    # feedback_autonomous-pilot-mandate.md가 파일로 존재하고 MEMORY.md 색인에 등재돼야
    # 모든 노드 기동 시 자율주행 권한·경계가 자동 주입된다(빠지면 매 단계 수동개입 대기로
    # 자율주행 무력화). 파일은 init-pack 임베드로, 색인 줄은 lock 하에 결정론 append로 수리.
    # ★실행 순서: C18(memory verify)보다 먼저 돌아야 같은 런에서 수리→정합 순이 된다.
    @staticmethod
    def _index_registered(itext):
        """색인 등재 판정 — javis_memory verify의 index_links와 동일 기준(링크 타깃,
        HTML 주석·코드펜스 제외). raw substring은 산문 언급을 등재로 오판한다(6차 R1)."""
        visible = re.sub(r"```.*?```", "", re.sub(r"<!--.*?-->", "", itext, flags=re.S),
                         flags=re.S)
        return ("](%s)" % AUTOPILOT_MEMORY_FILE) in visible

    def c25_autopilot_memory(self):
        cid = "C25.autopilot-memory"
        if self.skipped(cid):
            return
        mdir = os.path.join(pack_dir(), "memory")
        fpath = os.path.join(mdir, AUTOPILOT_MEMORY_FILE)
        idx = os.path.join(mdir, "MEMORY.md")
        fixed = []
        if not os.path.isfile(fpath):
            if self.fix and self.repair_via_init_pack() and os.path.isfile(fpath):
                fixed.append("메모리 파일 재설치")
            else:
                self.add(cid, FAIL, "memory/%s 없음 — `cys init-pack` 또는 --fix"
                         % AUTOPILOT_MEMORY_FILE)
                return
        # 본문 핀: 권한·경계의 실질이 비워지면(frontmatter만 잔존) 상주가 공수표다.
        try:
            ftext = open(fpath, encoding="utf-8", errors="replace").read()
        except OSError:
            self.add(cid, FAIL, "memory/%s 읽기 불가" % AUTOPILOT_MEMORY_FILE)
            return
        lost = [pin for pin in AUTOPILOT_MEMORY_PINS if pin not in ftext]
        if lost:
            self.add(cid, FAIL, "자율주행 메모리 본문 핀 소실: %s — `cys init-pack --force`"
                     "(사용자 수정 덮어씀 주의)로 템플릿 복원" % "·".join(lost))
            return
        try:
            itext = open(idx, encoding="utf-8", errors="replace").read()
        except OSError:
            self.add(cid, FAIL, "memory/MEMORY.md 읽기 불가 (C01 먼저)")
            return
        if not self._index_registered(itext):
            if not self.fix:
                self.add(cid, FAIL,
                         "MEMORY.md 색인에 자율주행 메모리 미등재(🔒상주 필수) — --fix로 등재 가능")
                return
            # javis_memory와 동일한 lock 규약(O_CREAT|O_EXCL + stale 회수)으로 색인 1줄
            # append — 결정론 도구의 기계 등재이지 LLM 손편집이 아니다.
            lock = idx + ".lock"
            acquired = False
            for _ in range(2):
                try:
                    fd = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
                    acquired = True
                    break
                except FileExistsError:
                    try:  # 죽은 프로세스의 만료 잠금(30초+)은 회수한다 — javis_memory와 동일
                        if time.time() - os.path.getmtime(lock) > 30:
                            os.unlink(lock)
                            continue
                    except OSError:
                        pass
                    break
            if not acquired:
                self.add(cid, FAIL, "MEMORY.md 잠금 경합(활성 lock) — 잠시 후 재실행. "
                         "30초+ 방치된 %s는 자동 회수된다" % lock)
                return
            try:
                with open(idx, "a", encoding="utf-8") as f:
                    if not itext.endswith("\n"):
                        f.write("\n")
                    f.write(AUTOPILOT_MEMORY_INDEX_LINE + "\n")
            finally:
                os.close(fd)
                os.unlink(lock)
            fixed.append("색인 등재(lock append)")
        if fixed:
            self.add(cid, FIXED, "자율주행 메모리 상주 수리: %s" % ", ".join(fixed))
        else:
            self.add(cid, PASS, "자율주행 메모리 파일 존재·본문 핀 건재·색인 등재(🔒상주)")

    # ── C26 영상 자동제작 스킬(cys-video-creator) 기본 탑재 ──
    # 비차단(영상 제작은 옵트인 능력) — 절대 FAIL 없음. 핵심(우리 스킬을 프로필에 심링크)은
    # 결정론 FIXED, 런타임 전제(도구·벤더스킬·키)는 WARN(정보). 자동화 경계는 C20/C24와 동일:
    # 우리 스킬 배선·벤더 스킬 설치는 기계(--fix), API 키 발급은 사람 단계.
    def c26_video_creator(self):
        cid = "C26.video-creator"
        if self.skipped(cid):
            return
        fixed, warns = [], []
        # (a) 우리 스킬이 pack에 임베드·설치됐는지(데몬 install 산출물) 확인
        missing = [s for s in VIDEO_SKILLS
                   if not os.path.isfile(os.path.join(pack_dir(), "skills", s, "SKILL.md"))]
        if missing:
            warns.append("pack 스킬 %d종 미설치(%s…) — init-pack 재실행 필요"
                         % (len(missing), missing[0]))
        # (b) 네이티브 Claude Code(/goal) 발견용 프로필 심링크 (기계 --fix)
        _home = os.path.expanduser("~")
        profiles = sorted(os.path.join(_home, d) for d in os.listdir(_home)
                          if (d == ".claude" or d.startswith(".claude-"))
                          and os.path.isdir(os.path.join(_home, d)))
        linked_profiles = 0
        for prof in profiles:
            sdir = os.path.join(prof, "skills")
            need = [s for s in VIDEO_SKILLS if s not in missing
                    and not self._symlink_ok(os.path.join(sdir, s),
                                             os.path.join(pack_dir(), "skills", s))]
            if not need:
                linked_profiles += 1
                continue
            if self.fix:
                try:
                    os.makedirs(sdir, exist_ok=True)
                    for s in need:
                        link = os.path.join(sdir, s)
                        target = os.path.join(pack_dir(), "skills", s)
                        if os.path.islink(link) or os.path.exists(link):
                            if os.path.islink(link):
                                os.unlink(link)
                            else:
                                continue  # 실디렉(사용자 보유) — 덮지 않음
                        os.symlink(target, link)
                    linked_profiles += 1
                    fixed.append("%s/skills ← 영상 스킬 심링크" % os.path.basename(prof))
                except OSError as e:
                    warns.append("%s 심링크 실패: %s" % (os.path.basename(prof), e))
            else:
                warns.append("%s/skills 영상 스킬 미배선(--fix로 심링크)" % os.path.basename(prof))
        # (c) 도구 — Node 22+·FFmpeg (WARN만, 영상 제작 시 필요)
        node_major = self._node_major()
        if node_major is None or node_major < 22:
            warns.append("Node 22+ 필요(HyperFrames 렌더) — 현재 %s"
                         % (node_major or "미설치"))
        if not shutil.which("ffmpeg"):
            warns.append("FFmpeg 미설치(HyperFrames·합성 필요)")
        # (d) 공식 벤더 스킬 — `npx skills add`는 cwd의 .agents/skills/에 프로젝트-로컬 설치라
        # 자동 실행하지 않는다(엉뚱한 cwd 오염 방지). 영상 작업 폴더에서 1회 실행 안내.
        warns.append("벤더 스킬은 영상 작업 폴더에서 1회: " + " · ".join(VIDEO_VENDOR_COMMANDS))
        # (e) 런타임 키 — 사람 단계(WARN 비차단)
        miss_keys = [k for k in VIDEO_RUNTIME_KEYS if not os.environ.get(k)]
        if miss_keys:
            warns.append("API 키 미설정: %s — 사람 단계(`export <KEY>=...`), 영상 제작 시 필요"
                         % ", ".join(miss_keys))
        # 판정: WARN 있으면 WARN(비차단), 없으면 PASS/FIXED
        detail = "영상 스킬 %d종 · 프로필 %d/%d 배선" % (
            len(VIDEO_SKILLS) - len(missing), linked_profiles, len(profiles) or 0)
        if fixed:
            detail += " · " + "; ".join(fixed)
        if warns:
            self.add(cid, WARN, detail + " · 전제: " + " | ".join(warns))
        else:
            self.add(cid, FIXED if fixed else PASS, detail)

    @staticmethod
    def _symlink_ok(link, target):
        return os.path.islink(link) and os.path.realpath(link) == os.path.realpath(target)

    @staticmethod
    def _node_major():
        node = shutil.which("node")
        if not node:
            return None
        try:
            out = subprocess.run([node, "-v"], capture_output=True,
                                 timeout=15).stdout.decode("utf-8", "replace")
            m = re.search(r"v(\d+)\.", out)
            return int(m.group(1)) if m else None
        except Exception:
            return None

    # ── C27 appbuild 웹/앱 빌드 스킬 + 코드선행 금지 hook (워커 필수) ──
    # 비차단(빌드는 옵트인)이되, 핵심은 결정론으로: 우리 20종을 프로필 심링크 + 게이트 hook을
    # PreToolUse로 등록(hook은 .appbuild 밖에선 fail-open이라 무관 작업 불간섭). 도구·키 불요
    # (cysjavis 자체 엔진 사용). FAIL 없음.
    @staticmethod
    def _appbuild_hook_registered(settings_path):
        try:
            data = json.load(open(settings_path, encoding="utf-8"))
        except (OSError, ValueError):
            return False
        if not isinstance(data, dict):
            return False
        for entry in data.get("hooks", {}).get("PreToolUse", []):
            for h in entry.get("hooks", []):
                if APPBUILD_HOOK in h.get("command", ""):
                    return True
        return False

    def _register_appbuild_hook(self, settings_path):
        """PreToolUse(Edit|Write|NotebookEdit)로 게이트 hook 등록. 성공=None, 실패=사유."""
        if os.path.islink(settings_path):
            return "symlink 거부: %s" % settings_path
        script = os.path.join(pack_dir(), "hooks", APPBUILD_HOOK)
        cmd = ("bash " if os.name == "nt" else "sh ") + script
        data = {}
        if os.path.isfile(settings_path):
            try:
                data = json.load(open(settings_path, encoding="utf-8"))
            except (OSError, ValueError) as e:
                return "기존 settings.json 파싱 실패 — 거부: %s" % e
            if not isinstance(data, dict):
                return "settings.json 루트가 객체가 아님 — 거부"
            backup = settings_path + ".bak-preflight"
            if not os.path.exists(backup):
                shutil.copy2(settings_path, backup)
        else:
            d = os.path.dirname(settings_path)
            if d:
                os.makedirs(d, exist_ok=True)
        arr = data.setdefault("hooks", {}).setdefault("PreToolUse", [])
        arr.append({"matcher": "Edit|Write|NotebookEdit",
                    "hooks": [{"type": "command", "command": cmd}]})
        tmp = settings_path + ".tmp"
        open(tmp, "w", encoding="utf-8").write(
            json.dumps(data, ensure_ascii=False, indent=2))
        os.replace(tmp, settings_path)
        return None

    # ── 역할-능력 GATE hook 등록 (appbuild-gate 등록과 동형 — PreToolUse 차단 클래스) ──
    # role-capability-gate.sh 를 PreToolUse(matcher Edit|Write|NotebookEdit|MultiEdit|Bash)로 등록.
    # 멱등: command 경로에 스크립트명이 들어간 PreToolUse 엔트리가 이미 있으면 재등록 안 함.
    @staticmethod
    def _capgate_hook_registered(settings_path):
        try:
            data = json.load(open(settings_path, encoding="utf-8"))
        except (OSError, ValueError):
            return False
        if not isinstance(data, dict):
            return False
        for entry in data.get("hooks", {}).get("PreToolUse", []):
            for h in entry.get("hooks", []):
                if CAPGATE_HOOK in h.get("command", ""):
                    return True
        return False

    def _register_capgate_hook(self, settings_path):
        """PreToolUse(Edit|Write|NotebookEdit|MultiEdit|Bash)로 능력 GATE hook 등록.
        성공=None, 실패=사유. _register_appbuild_hook 과 동일 규약(symlink 거부·파싱실패 거부·백업·원자적)."""
        if os.path.islink(settings_path):
            return "symlink 거부: %s" % settings_path
        script = os.path.join(pack_dir(), "hooks", CAPGATE_HOOK)
        cmd = ("bash " if os.name == "nt" else "sh ") + script
        data = {}
        if os.path.isfile(settings_path):
            try:
                data = json.load(open(settings_path, encoding="utf-8"))
            except (OSError, ValueError) as e:
                return "기존 settings.json 파싱 실패 — 거부: %s" % e
            if not isinstance(data, dict):
                return "settings.json 루트가 객체가 아님 — 거부"
            backup = settings_path + ".bak-preflight"
            if not os.path.exists(backup):
                shutil.copy2(settings_path, backup)
        else:
            d = os.path.dirname(settings_path)
            if d:
                os.makedirs(d, exist_ok=True)
        arr = data.setdefault("hooks", {}).setdefault("PreToolUse", [])
        arr.append({"matcher": CAPGATE_HOOK_MATCHER,
                    "hooks": [{"type": "command", "command": cmd}]})
        tmp = settings_path + ".tmp"
        open(tmp, "w", encoding="utf-8").write(
            json.dumps(data, ensure_ascii=False, indent=2))
        os.replace(tmp, settings_path)
        return None

    # ── C28 자기교정·영속성 hook 등록 헬퍼 (event 일반화) ──
    @staticmethod
    def _event_hook_registered(settings_path, event, script_name):
        """event 에 pack 경로의 script_name 이 등록돼 있나 (구 .config 경로는 미인정)."""
        try:
            data = json.load(open(settings_path, encoding="utf-8"))
        except (OSError, ValueError):
            return False
        if not isinstance(data, dict):
            return False
        pd = pack_dir()
        for entry in data.get("hooks", {}).get(event, []):
            for h in entry.get("hooks", []):
                cmd = h.get("command", "")
                if script_name in cmd and pd in cmd:
                    return True
        return False

    def _register_event_hook(self, settings_path, event, script_name, matcher=None):
        """event 에 pack/hooks/script_name 등록. 성공=None, 실패=사유. 멱등은 호출부.
        _register_appbuild_hook 과 동일 규약(symlink 거부·파싱실패 거부·백업·원자적)."""
        if os.path.islink(settings_path):
            return "symlink 거부: %s" % settings_path
        script = os.path.join(pack_dir(), "hooks", script_name)
        cmd = ("bash " if os.name == "nt" else "sh ") + script
        data = {}
        if os.path.isfile(settings_path):
            try:
                data = json.load(open(settings_path, encoding="utf-8"))
            except (OSError, ValueError) as e:
                return "기존 settings.json 파싱 실패 — 거부: %s" % e
            if not isinstance(data, dict):
                return "settings.json 루트가 객체가 아님 — 거부"
            backup = settings_path + ".bak-preflight"
            if not os.path.exists(backup):
                shutil.copy2(settings_path, backup)
        else:
            d = os.path.dirname(settings_path)
            if d:
                os.makedirs(d, exist_ok=True)
        entry = {"hooks": [{"type": "command", "command": cmd}]}
        if matcher is not None:
            entry["matcher"] = matcher
        data.setdefault("hooks", {}).setdefault(event, []).append(entry)
        tmp = settings_path + ".tmp"
        open(tmp, "w", encoding="utf-8").write(
            json.dumps(data, ensure_ascii=False, indent=2))
        os.replace(tmp, settings_path)
        return None

    def c27_appbuild(self):
        cid = "C27.appbuild"
        if self.skipped(cid):
            return
        fixed, warns = [], []
        # (a) 우리 스킬이 pack에 설치됐는지
        missing = [s for s in APPBUILD_SKILLS
                   if not os.path.isfile(os.path.join(pack_dir(), "skills", s, "SKILL.md"))]
        if missing:
            warns.append("pack 스킬 %d종 미설치(%s…) — init-pack 재실행"
                         % (len(missing), missing[0]))
        # (b) 프로필 심링크 (네이티브/goal 발견 — C26과 동일 규약)
        _home = os.path.expanduser("~")
        profiles = sorted(os.path.join(_home, d) for d in os.listdir(_home)
                          if (d == ".claude" or d.startswith(".claude-"))
                          and os.path.isdir(os.path.join(_home, d)))
        linked = 0
        for prof in profiles:
            sdir = os.path.join(prof, "skills")
            need = [s for s in APPBUILD_SKILLS if s not in missing
                    and not self._symlink_ok(os.path.join(sdir, s),
                                             os.path.join(pack_dir(), "skills", s))]
            if not need:
                linked += 1
                continue
            if self.fix:
                try:
                    os.makedirs(sdir, exist_ok=True)
                    for s in need:
                        link = os.path.join(sdir, s)
                        if os.path.islink(link):
                            os.unlink(link)
                        elif os.path.exists(link):
                            continue
                        os.symlink(os.path.join(pack_dir(), "skills", s), link)
                    linked += 1
                    fixed.append("%s/skills ← appbuild 심링크" % os.path.basename(prof))
                except OSError as e:
                    warns.append("%s 심링크 실패: %s" % (os.path.basename(prof), e))
            else:
                warns.append("%s/skills appbuild 미배선(--fix)" % os.path.basename(prof))
        # (c) 게이트 hook 존재·실행권한
        hook_path = os.path.join(pack_dir(), "hooks", APPBUILD_HOOK)
        if not os.path.isfile(hook_path):
            warns.append("게이트 hook 미설치 — init-pack 재실행")
        elif os.name == "posix":
            mode = os.stat(hook_path).st_mode
            if not mode & stat.S_IXUSR and self.fix:
                os.chmod(hook_path, mode | 0o755)
                fixed.append("게이트 hook 실행권한")
        # (d) PreToolUse 게이트 hook 등록 (결정론 — .appbuild 밖 fail-open이라 안전)
        if os.path.isfile(hook_path):
            targets = discover_claude_settings() or [
                os.path.join(os.path.expanduser("~"), ".claude", "settings.json")]
            reg = 0
            for t in targets:
                if self._appbuild_hook_registered(t):
                    reg += 1
                    continue
                if self.fix:
                    err = self._register_appbuild_hook(t)
                    if err:
                        warns.append("hook 등록 실패(%s): %s" % (os.path.basename(t), err))
                    else:
                        reg += 1
                        fixed.append("%s에 게이트 hook 등록" % os.path.basename(t))
                else:
                    warns.append("게이트 hook 미등록(--fix)")
        # 판정 (FAIL 없음)
        detail = "appbuild 스킬 %d종 · 프로필 %d/%d 배선 · 코드선행 금지 hook" % (
            len(APPBUILD_SKILLS) - len(missing), linked, len(profiles) or 0)
        if fixed:
            detail += " · " + "; ".join(fixed)
        if warns:
            self.add(cid, WARN, detail + " · " + " | ".join(warns))
        else:
            self.add(cid, FIXED if fixed else PASS, detail)

    def c28_self_correction(self):
        cid = "C28.self-correction"
        if self.skipped(cid):
            return
        fixed, warns = [], []
        # (a) hook 스크립트 4종 + javis_reflect.py 존재·실행권한
        rels = [os.path.join("hooks", s) for s, _ in SELFCORR_HOOKS]
        rels.append(os.path.join("bin", "javis_reflect.py"))
        for rel in rels:
            p = os.path.join(pack_dir(), rel)
            if not os.path.isfile(p):
                if self.fix and self.repair_via_init_pack() and os.path.isfile(p):
                    pass
                else:
                    warns.append("%s 미설치 — init-pack 재실행" % rel)
                    continue
            if os.name == "posix":
                mode = os.stat(p).st_mode
                if not mode & stat.S_IXUSR and self.fix:
                    os.chmod(p, mode | 0o755)
                    fixed.append("%s 실행권한" % os.path.basename(p))
        # (b) 이벤트별 등록 (멱등 — 구 .config 경로는 미인정이라 패키지 경로로 신규 등록)
        targets = discover_claude_settings() or [
            os.path.join(os.path.expanduser("~"), ".claude", "settings.json")]
        for t in targets:
            for script_name, events in SELFCORR_HOOKS:
                if not os.path.isfile(os.path.join(pack_dir(), "hooks", script_name)):
                    continue
                for event, matcher in events:
                    if self._event_hook_registered(t, event, script_name):
                        continue
                    if self.fix:
                        err = self._register_event_hook(t, event, script_name, matcher)
                        if err:
                            warns.append("%s/%s 등록 실패: %s"
                                         % (os.path.basename(t), event, err))
                        else:
                            fixed.append("%s←%s(%s)"
                                         % (os.path.basename(t), script_name, event))
                    else:
                        warns.append("%s %s 미등록(--fix)"
                                     % (os.path.basename(t), script_name))
        detail = "자기교정·영속성 hook(inject·save·reflect-scan·commit-nudge) 4종 + reflect 엔진"
        if fixed:
            shown = "; ".join(fixed[:6]) + (" …+%d" % (len(fixed) - 6) if len(fixed) > 6 else "")
            detail += " · " + shown
        if warns:
            self.add(cid, WARN, detail + " · " + " | ".join(warns[:6]))
        else:
            self.add(cid, FIXED if fixed else PASS, detail)

    def c29_harness_engineering(self):
        cid = "C29.harness-engineering"
        if self.skipped(cid):
            return
        fixed, warns = [], []
        # (a) 우리 스킬이 pack에 설치됐는지 (build.rs 임베드 → init-pack 산출물)
        missing = [s for s in HARNESS_SKILLS
                   if not os.path.isfile(os.path.join(pack_dir(), "skills", s, "SKILL.md"))]
        if missing:
            warns.append("pack 스킬 미설치(%s) — init-pack 재실행" % ", ".join(missing))
        # (b) 프로필 심링크 (네이티브 스킬 발견 — C26/C27과 동일 규약)
        _home = os.path.expanduser("~")
        profiles = sorted(os.path.join(_home, d) for d in os.listdir(_home)
                          if (d == ".claude" or d.startswith(".claude-"))
                          and os.path.isdir(os.path.join(_home, d)))
        linked = 0
        for prof in profiles:
            sdir = os.path.join(prof, "skills")
            need = [s for s in HARNESS_SKILLS if s not in missing
                    and not self._symlink_ok(os.path.join(sdir, s),
                                             os.path.join(pack_dir(), "skills", s))]
            if not need:
                linked += 1
                continue
            if self.fix:
                try:
                    os.makedirs(sdir, exist_ok=True)
                    for s in need:
                        link = os.path.join(sdir, s)
                        if os.path.islink(link):
                            os.unlink(link)
                        elif os.path.exists(link):
                            continue  # 실디렉(사용자 보유) — 덮지 않음
                        os.symlink(os.path.join(pack_dir(), "skills", s), link)
                    linked += 1
                    fixed.append("%s/skills ← 하네스 스킬 심링크" % os.path.basename(prof))
                except OSError as e:
                    warns.append("%s 심링크 실패: %s" % (os.path.basename(prof), e))
            else:
                warns.append("%s/skills 하네스 스킬 미배선(--fix)" % os.path.basename(prof))
        # 판정 (FAIL 없음 — 하네스 운영은 옵트인 능력)
        detail = "하네스 스킬 %d종 · 프로필 %d/%d 배선" % (
            len(HARNESS_SKILLS) - len(missing), linked, len(profiles) or 0)
        if fixed:
            detail += " · " + "; ".join(fixed)
        if warns:
            self.add(cid, WARN, detail + " · " + " | ".join(warns))
        else:
            self.add(cid, FIXED if fixed else PASS, detail)

    # ── C30 git 결정론 점검 (오너 2026-06-14 — git 온보딩) ──
    # git은 기여자 clone·harness-creator(C21) 툴체인 자동설치·RSI 자기개선 push에 필요하다.
    # 일반 .dmg 사용자 기본 기능엔 불필요 → 부재는 FAIL이 아니라 WARN(기능별 필수).
    def c30_git(self):
        cid = "C30.git"
        if self.skipped(cid):
            return
        p = shutil.which("git")
        if p:
            self.add(cid, PASS, "%s (기여자 clone·harness-creator·RSI 자기개선에 사용)" % p)
        else:
            self.add(cid, WARN,
                     "git 미설치 — 기여자 clone·harness-creator(C21)·RSI 자기개선이 막힌다. "
                     "설치: macOS `xcode-select --install`(또는 brew install git) · "
                     "Windows git-scm.org · Linux `apt/dnf install git`. "
                     "(일반 .dmg 사용자 기본기능엔 불필요 — 기능별 필수)")

    # ── C31 config dir 격리 + 오염 감지 (박사님 2026-06-15) ──
    # cys 마스터는 전용 CLAUDE_CONFIG_DIR(~/.cys/claude)로 격리 기동돼 사용자 ~/.claude 의
    # 외부 터미널 체계·구 지침 오염에 영향받지 않는다. 이 체크는 ①격리 라우터 설치 확인 ②사용자
    # 프로필 오염 감지(경고만 — 자동삭제 절대 안 함, 사용자 데이터 불가침)다.
    def c31_config_isolation(self):
        cid = "C31.config-isolation"
        if self.skipped(cid):
            return
        home = os.path.expanduser("~")
        cfg = os.path.join(os.path.dirname(os.path.normpath(pack_dir())), "claude")
        router = os.path.join(cfg, "CLAUDE.md")
        if not os.path.isfile(router):
            self.add(cid, WARN,
                     "cys 전용 config dir 라우터 부재(%s) — `cys init-pack` 재실행 권장 "
                     "(격리 없으면 사용자 ~/.claude 오염에 노출)" % router)
            return
        # 사용자 ~/.claude* 에 외부 터미널 체계 명령을 쓰는 구체계/구 지침 잔재 감지 (패턴은 레거시 식별자 유지)
        contaminated = []
        try:
            entries = [n for n in os.listdir(home)
                       if n == ".claude" or n.startswith(".claude-")]
        except OSError:
            entries = []
        cmux_cmd = re.compile(r"cmux (send|launch|new-split|identify|list-workspaces|notify)|cmux\.app")
        for n in entries:
            for fn in ("CLAUDE.md", "soul.md", "CSO_DIRECTIVE.md", "MASTER_DIRECTIVE.md"):
                p = os.path.join(home, n, fn)
                try:
                    t = open(p, encoding="utf-8", errors="replace").read()
                except OSError:
                    continue
                # cys 치환 선언("cmux 아님"/"치환")이 있으면 신체계 — 오염 아님
                if cmux_cmd.search(t) and ("cmux 아님" not in t) and ("치환" not in t):
                    contaminated.append(p)
        if contaminated:
            self.add(cid, WARN,
                     "사용자 프로필에 외부 터미널 체계/구 지침 %d건 감지 — cys는 전용 config dir로 격리돼 "
                     "영향 없으나, 정리하려면 **백업 후 직접 제거**(cys는 자동삭제 안 함): %s"
                     % (len(contaminated), ", ".join(contaminated[:3])))
            return
        self.add(cid, PASS, "격리 config dir 라우터 설치됨 · 사용자 프로필 외부 체계 오염 없음")

    def run(self):
        self.c01_pack_dir()
        self.c02_directives()
        self.c03_content_pins()
        self.c04_soul()
        self.c05_agents()
        self.c06_json_files()
        self.c07_hook_script()
        self.c08_hook_registered()
        self.c09_round_core()
        self.c10_todo_files()
        self.c11_cys_binary()
        self.c12_daemon()
        self.c13_claude_md()
        self.c14_self()
        self.c15_report_tool()
        self.c16_report_schedule()
        self.c17_route_engine()
        # C25를 C18보다 먼저: C25의 --fix(파일 설치·색인 등재)가 정합을 만든 뒤 C18이
        # verify해야 같은 런에서 FAIL/FIXED 플랩(NOT READY 헛사이클)이 없다(6차 R1).
        self.c25_autopilot_memory()
        self.c18_memory_engine()
        self.c19_orchestra_engine()
        self.c20_nlm_sot()
        self.c21_harness_creator()
        self.c22_work_skills()
        self.c23_governance_conflict()
        self.c24_korean_law_mcp()
        self.c26_video_creator()
        self.c27_appbuild()
        self.c28_self_correction()
        self.c29_harness_engineering()
        self.c30_git()
        self.c31_config_isolation()
        self.c32_statusline()
        self.c33_event_hooks()
        self.c34_registry()
        self.c35_select()
        self.c36_verdict()
        self.c37_adr_engine()
        self.c38_silent_failure_catalog()
        self.c39_prereq_orphan_lint()
        self.c40_workflow_manifest()
        self.c41_reference_integrity()
        self.c42_cleanroom()
        self.c43_verdict_literals()
        self.c44_kind_enum_parity()
        self.c45_statedb()
        self.c46_session_ensure()
        self.c47_capability_guard()
        self.c48_governance_hardening()
        self.c49_onboarding_catalog()
        self.c50_skill_requirements()
        self.c51_verdict_schema_sot()
        return self.results


def main():
    ap = argparse.ArgumentParser(description="CYSJavis 결정론 부트 프리플라이트")
    ap.add_argument("--fix", action="store_true", help="수리 가능한 항목 자동 수리")
    ap.add_argument("--json", action="store_true", help="JSON 출력")
    ap.add_argument("--skip", action="append", default=[], metavar="ID",
                    help="해당 검사 건너뜀 (예: --skip C12.daemon)")
    args = ap.parse_args()

    pf = Preflight(fix=args.fix, skips=args.skip)
    results = pf.run()
    fails = sum(1 for r in results if r["status"] == FAIL)
    warns = sum(1 for r in results if r["status"] == WARN)

    if args.json:
        print(json.dumps(
            {"ok": fails == 0, "fails": fails, "warns": warns,
             "pack_dir": pack_dir(), "checks": results},
            ensure_ascii=False, indent=2,
        ))
    else:
        for r in results:
            print("[%s] %s — %s" % (r["status"], r["id"], r["detail"]))
        print("─" * 60)
        verdict = "READY (프로젝트 시작 준비 완료)" if fails == 0 else "NOT READY"
        print("preflight: %s — FAIL %d · WARN %d · 검사 %d"
              % (verdict, fails, warns, len(results)))
        if fails:
            print("FAIL 항목을 수리하고 재실행하라. 이 출력 외의 추론으로 READY를 선언하지 마라.")
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
