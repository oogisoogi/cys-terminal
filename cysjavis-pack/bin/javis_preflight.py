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


def pack_dir():
    """pack 위치 결정 — src/pack.rs pack_dir()의 4단 폴백을 그대로 미러링한다."""
    for key in ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_JARVIS_DIR"):
        v = os.environ.get(key, "")
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys/pack")


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
