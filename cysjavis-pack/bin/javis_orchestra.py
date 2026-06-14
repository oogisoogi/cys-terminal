#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_orchestra — LLM 오케스트레이션의 결정론 도구 (절대지침 4차: LLM orchestrating 앵커).

master가 (a) "4개 노드 다 떴나"를 눈대중 판단, (b) 리뷰 프롬프트에 제약을 빠뜨림,
(c) 라운드 번호·완료조건을 머리로 셈 — 이 세 가지는 결정론으로 환원 가능하다. 이 도구가
그 사실을 산출한다(LLM 자연어 재추론 금지 — 출력만이 사실).

서브커맨드:
  check                         4종 의무 노드(cso·worker·reviewer-gemini·reviewer-codex)
                                생존을 cys status로 판정. exit 0=4종 생존, 1=부재 존재.
  review-prompt --task T --scope S [--reviewer gemini|codex] [--round N] [--success X]
                                REVIEWER_DIRECTIVE §2 제약 + 형식 + 회신 채널을 항상 포함한
                                리뷰 의뢰 프롬프트를 출력(제약 누락 구조 차단). --success는
                                구현 위임과 동일한 평가 기준을 리뷰어에게도 투입(N6 영상 양방향 —
                                "구현할 때도 먹이고 리뷰할 때도 똑같이"). 생략 시 출력 바이트 동일.
  task-prompt  --task T --scope S [--success C] [--to ROLE]
                                위임 티켓 생성(절대지침 5차 work management 앵커):
                                ①위임 직전 대상 노드 생존을 결정론 확인(미기동=티켓 미출력 —
                                "워커 정상 작동 확인 후 작업 지시") ②WORKER §3
                                절대 강조 4규칙(품질·할루시네이션 방지·의도 합의·요약 금지)을
                                항상 주입 — 추출분이 마커 불완전하면 하드 폴백으로 강등·경고
                                (약화 전파·강조 누락 구조 차단).
                                exit: 0=티켓 출력(stdout은 티켓만, 경고는 stderr) /
                                1=대상 미기동 / 2=확인 불가(데몬 다운·역할명 위반).
  phase-plan   --task T --phases "p1;p2;p3" --scope S [--success X] [--to ROLE]
                                Task를 세미콜론 분리 Phase로 분해해 각 Phase의 자기완결 티켓
                                (P1/P2/… · build_task_ticket 재사용으로 절대 강조 4규칙 포함)을
                                출력하고 round/PHASE-<task>.json 인덱스(상태 pending) 기록.
                                각 Phase는 독립 세션이 "이것만 보고도" 완수하게 자기완결(영상 N6).
                                코드는 claude -p raw subprocess를 띄우지 않는다 — Workflow
                                pipeline·cys 워커 순차 위임으로 실행(스킬 안내).
                                exit: 0=출력 / 2=phases 비었거나 역할명 위반.
  round-init   --task T                       라운드 장부 생성
  round-log    --task T --round N --evaluator E [--score X --verdict V | --from-cmd CMD]
                                라운드 기록 append. --from-cmd는 기계검증 명령을 직접 실행해
                                exit code로 verdict 자동 기록(machine 평가자 규약 — 전사 금지).
                                exit: 0=기록(검증 통과 포함) / 1=기록됨·기계검증 실패
                                (기록 성공≠검증 통과 — 판정의 단일 진실은 gate-status).
  round-status --task T                       현재 라운드·10R 도달·최근 점수 결정론 판정
  gate-status  --task T [--round N]           자율주행(앵커6 축1) 게이트 4자 수렴 결정론 판정:
                                해당 라운드에 gemini·codex·master·machine 4평가자의 승인
                                (PASS/수렴/approve/ok/green 접두) 기록이 전부 있어야 CONVERGED.
                                exit 0=수렴(다음 단계 자동 착수 가) / 1=미수렴(사유 출력).
  next-action                   자율주행(앵커6 축3) 다음 액션 결정론 추출: pack/round/
                                SESSION_STATE.md '## 다음 액션' 섹션의 첫 미완 항목을 출력.
                                exit 0=항목 있음 / 1=큐 비음(전 작업 완료 — 정지·오너 보고)
                                / 2=SESSION_STATE 부재(신규 시작 — 오너 지시 대기).

의존성: 파이썬 표준 라이브러리 + PATH의 cys(check만 필요).
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys

# 4차 앵커4-1: 프로젝트 상주 의무 노드(grok은 선택). check가 이 4종 생존을 판정한다.
REQUIRED_ROLES = ["cso", "worker", "reviewer-gemini", "reviewer-codex"]
OPTIONAL_ROLES = ["reviewer-grok"]
MAX_ROUNDS = 10  # 앵커4 5-8: 맥킨지급 도달 또는 10R 완료 시 멈춤


def pack_dir():
    for key in ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_JARVIS_DIR"):
        v = os.environ.get(key, "")
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys/pack")


def cys_status():
    cys = shutil.which("cys")
    if not cys:
        return None
    try:
        r = subprocess.run([cys, "status", "--json"], capture_output=True, timeout=10)
        if r.returncode != 0:
            return None
        return json.loads(r.stdout.decode("utf-8", "replace"))
    except Exception:
        return None


# set-status 자기보고 신선도 임계(초). 이 안에 자기보고가 있으면 '살아 일하는 중'으로 본다.
STATUS_FRESH_SECS = 600


def live_roles(status):
    """role → alive(bool). 순수 함수(입력 status 만으로 판정).

    판정: agent_alive OR set-status 자기보고가 신선(age<=STATUS_FRESH_SECS·state 존재).
    ★부트스트랩 FAILURE 3 재발방지(2026-06-13): launch-agent 주입 실패로 agent 메타데이터가
    None 이거나 노드가 node 래퍼로 떠 agent_alive 가 구조적 false-negative 여도, 노드의
    set-status 자기보고(=디렉티브를 읽고 각성한 증거)를 결정론 신호로 인정해 '각성했는데 미기동'
    오판을 차단한다.
    ★단, '프로세스 존재'만으로는 생존 인정하지 않는다(codex R1 적대검증 결함5 반영): 빈 CLI(디렉티브
    미수신)를 READY 로 오인증하면 false-negative 가 false-positive 로 바뀐다. 부트 성공의 계약은
    어디까지나 set-status ack 다. 프로세스 탐침은 stuck pane 회수 판단(javis_boot_node.py --reclaim)
    에만 쓴다."""
    out = {}
    for s in status.get("surfaces", []):
        role = s.get("role")
        if not role or s.get("exited"):
            continue
        if s.get("agent_alive"):
            out[role] = True
            continue
        st = s.get("status") or {}
        age = st.get("age_secs")
        if isinstance(age, (int, float)) and age <= STATUS_FRESH_SECS and st.get("state"):
            out[role] = True
    return out


def _quiet_alive_roles(status, roles):
    """미확정 role 중 '생존추정'(각성이력+프로세스)인 것 → {role: True}.
    ★생존 술어는 javis_boot_node.quiet_but_alive 단일 정의를 공유한다(codex R2 결함2 — cmd_check 와
    reclaim 이 같은 상태를 반대로 해석하던 중복 로직 제거). status 를 surface_ref 에 결박해
    litter/exited row·과거이력 오인을 차단한다."""
    out = {}
    try:
        import javis_boot_node as _bn
    except Exception:
        return out
    for role in roles:
        if _bn.quiet_but_alive(status, role):
            out[role] = True
    return out


# ── check: 4종 의무 노드 생존 판정 ──
def cmd_check(args):
    status = cys_status()
    if status is None:
        print("[orchestra check] cys status 수집 실패(데몬 미가동?) — `cys ping` 확인 후 재실행")
        return 2
    alive = live_roles(status)
    # 각성 이력 있는 idle 노드(set-status 노후화·agent_alive None 으로 굳음)만 '생존추정'으로 보강.
    # ★프로세스 단독 인증 아님(각성이력=status.state 필수·surface_ref 결박) — codex R1 결함5·R2 결함1·5 정합.
    still_missing = [r for r in REQUIRED_ROLES if not alive.get(r)]
    estimated = _quiet_alive_roles(status, still_missing) if still_missing else {}
    alive.update(estimated)
    print("LLM orchestrating 노드 점검 (4종 의무 + grok 선택):")
    missing = []
    for r in REQUIRED_ROLES:
        if alive.get(r):
            if estimated.get(r):
                # fresh 각성이 아니라 '각성이력+프로세스' 추정 — 재각성(헬퍼) 권장 신호.
                print("  ✓ %s — 생존추정(set-status 노후·프로세스 생존 · 재각성 권장)" % r)
            else:
                print("  ✓ %s — 생존" % r)
        else:
            print("  ✗ %s — 미기동" % r)
            missing.append(r)
    for r in OPTIONAL_ROLES:
        print("  %s %s — %s" % ("✓" if alive.get(r) else "·", r,
                                "생존" if alive.get(r) else "미설치/미기동(선택)"))
    if missing:
        print("종합: 필수 %d/%d 생존 — 부재: %s → `cys boot`로 기동하라"
              % (len(REQUIRED_ROLES) - len(missing), len(REQUIRED_ROLES), ", ".join(missing)))
        return 1
    print("종합: 4종 의무 노드 전부 생존 — LLM orchestrating READY")
    return 0


# ── review-prompt: 제약을 항상 포함한 리뷰 의뢰 프롬프트 ──
def extract_constraints():
    """REVIEWER_DIRECTIVE §2 '엄격 제약' 항목을 디렉티브에서 동적 추출(진실원천)."""
    p = os.path.join(pack_dir(), "directives", "REVIEWER_DIRECTIVE.md")
    try:
        text = open(p, encoding="utf-8", errors="replace").read()
    except OSError:
        return None
    # "## 2. 엄격 제약" 섹션의 '- ' 불릿만 추출 (다음 '## ' 전까지)
    m = re.search(r"##\s*2\.\s*엄격 제약.*?\n(.*?)(?:\n##\s|\Z)", text, re.S)
    if not m:
        return None
    bullets = [ln.strip() for ln in m.group(1).splitlines() if ln.strip().startswith("- ")]
    return bullets or None


def cmd_review_prompt(args):
    bullets = extract_constraints()
    if not bullets:
        # 디렉티브 추출 실패 시에도 제약 누락은 허용 불가 — REVIEWER_DIRECTIVE §2 원문과
        # 동기화한 최소 제약을 하드 폴백(잘림 없이 전문 보존).
        bullets = [
            "- 지정된 파일/범위만 검토한다. 무관 저장소·파일 배회 금지, 도구 남용 금지.",
            "- 서버·장시간 프로세스를 띄우지 않는다. 필요하면 의뢰자에게 요청한다.",
            "- 검토 대상을 직접 수정하지 않는다(의견 제시가 기본). 직접 생성·수정 의뢰를 "
            "받은 경우에만 계약(파일·범위)을 선합의하고 수행한다.",
        ]
    rnd = args.round
    lines = []
    lines.append("[리뷰 의뢰 — 엄격 제약 준수 · 지정 범위만]")
    lines.append("검토 범위(이 파일/범위만, 무관 파일·repo 배회 금지): %s" % args.scope)
    lines.append("과업: %s" % args.task)
    # 평가 기준 양방향(영상 N3 — "구현할 때도 먹이고 리뷰할 때도 똑같이 먹임"): success가 있으면
    # 구현 위임(task-prompt --success)과 동일한 기준을 리뷰어에게도 투입한다. 없으면 라인 생략
    # (회귀 0 — 기존 출력 바이트 동일).
    if getattr(args, "success", None):
        lines.append("평가 기준(구현 위임과 동일 기준 — 이 기준 대비 채점하라): %s" % args.success)
    lines.append("")
    lines.append("엄격 제약 (REVIEWER_DIRECTIVE §2 — 위반 금지):")
    lines.extend("  " + b for b in bullets)
    lines.append("")
    lines.append("리뷰 형식: [문제점] [논쟁점] [다음 단계 조언] — 각 지적에 파일:라인 또는 구체 근거.")
    lines.append("근거 없는 인상비평·칭찬만 하는 리뷰 금지. 결함을 찾는 것이 직무다.")
    if rnd and rnd > 1:
        lines.append("라운드 %d: 직전 산출물을 해당 분야 최고 전문가 관점으로 평가하고 "
                     "**직전 점수 +10%%** 목표로 본다. 단순 코드수정이 아니라 재귀적 개선 관점으로." % rnd)
    lines.append("회신: `cys send --queued --to master \"[리뷰] ...\"` (자동 Return 배달 — "
                 "타이핑 가드 안전·send-key 불필요).")
    print("\n".join(lines))
    return 0


# ── task-prompt: 생존 게이트 + 절대 강조 4규칙을 항상 포함한 위임 티켓 ──
# 4규칙 무결성 마커 — 추출분이 이 전부를 포함해야 '완전한 4규칙'으로 인정한다.
# (부분 잘림·약화된 디렉티브가 티켓으로 전파되는 silent failure를 구조 차단 — 적대 검증 R1)
RULE_MARKERS = ("품질 절대우선", "할루시네이션 방지", "hallucination-guard", "몽상",
                "Garbage-in", "grill-me", "합의에 이를 때까지", "요약·압축 절대 금지",
                "전문용어·약호", "길이는 원문 수준", "충돌 시 상위 기준 절대 우선")


def extract_rules_from_text(text):
    """§N '절대 강조' 섹션의 불릿을 추출(순수 함수 — self-test가 밀폐 검증).

    - 헤더는 줄 시작 '## N.' + '절대 강조' (번호 하드코딩 안 함 — 절 번호 변경에 견딤)
    - 불릿 연속줄('- '로 시작하지 않는 들여쓰기 줄)은 직전 불릿에 합류 — 개행 wrap 잘림 방지
    - RULE_MARKERS 전부 포함해야 반환. 하나라도 빠지면 None(=폴백) — 약화 전파 차단
    """
    m = re.search(r"(?m)^##\s*\d+\.[^\n]*절대 강조[^\n]*\n(.*?)(?:\n##\s|\Z)", text, re.S)
    if not m:
        return None
    bullets = []
    for ln in m.group(1).splitlines():
        s = ln.strip()
        if s.startswith("- "):
            bullets.append(s)
        elif s and bullets:
            bullets[-1] += " " + s  # 연속줄 합류
    if not bullets:
        return None
    joined = "\n".join(bullets)
    if any(mark not in joined for mark in RULE_MARKERS):
        return None  # 부분 추출·약화 — 폴백이 안전
    return bullets


def extract_worker_rules():
    """WORKER_DIRECTIVE §'절대 강조 4규칙'을 디렉티브에서 동적 추출(진실원천)."""
    p = os.path.join(pack_dir(), "directives", "WORKER_DIRECTIVE.md")
    try:
        text = open(p, encoding="utf-8", errors="replace").read()
    except OSError:
        return None
    return extract_rules_from_text(text)


# WORKER §3 원문과 동기화한 하드 폴백(잘림 없이 전문 보존) — 추출 실패 시에도
# 절대 강조 4규칙 누락은 허용 불가(절대지침 5차: "task 시행을 명령할 때마다 절대 강조").
FALLBACK_RULES = [
    "- a) **품질 절대우선**: 조사의 깊이·폭·정확도가 절대 기준이다. 속도·토큰·편의는 "
    "이유가 될 수 없다.",
    "- b) **할루시네이션 방지**: 출처·근거·논리오류 분석·팩트체크가 필수인 작업·판단에는 "
    "전담 sub-skill(`cys skill show hallucination-guard`)을 반드시 사용해 검증 엄밀성·평가 "
    "신뢰성·환각 안전장치를 확보한다. 과장·거짓 확신·현실감 없는 출력 금지, 몽상·망상을 "
    "촉진하는 말 절대 금지. Garbage-in 차단 — 토대가 오염되면 아무리 다듬어도 거짓만 정교해진다.",
    "- c) **의도 합의**: 받은 지시의 의도 파악이 불충분하면 추측 진행 금지 — grill-me 스킬"
    "(`cys skill show grill-me`) 등으로 의뢰자(master)와 합의에 이를 때까지 질문을 반복한다.",
    "- d) **요약·압축 절대 금지**: 최종 결과물은 일반인도 이해하고 읽기 편하게 첨삭하되, 모든 "
    "분석·수치·표·단서를 하나도 빠뜨리지 않는다. 전문용어·약호·내부 검증 표시만 쉬운 말로 "
    "풀고 길이는 원문 수준을 유지한다.",
    "- **게이트**: 충돌 시 상위 기준 절대 우선. ②(b 할루시네이션 방지·검증)가 흔들리면 "
    "①③(그 위에 쌓는 나머지 실행)을 중단하고 master에 보고한다 — 토대 오염 위에 쌓지 마라.",
]


def build_task_ticket(task, scope, success, to_role, rules, output_format=None):
    """위임 티켓 본문 생성. rules는 필수 — 호출자가 추출 성패를 알고 명시 주입한다
    (기본값 경유의 무경고 폴백 경로 제거 · self-test는 rules 주입으로 밀폐 검증)."""
    bullets = rules
    lines = []
    lines.append("[작업 위임 — 절대 강조 4규칙 포함 · work management 앵커]")
    lines.append("작업: %s" % task)
    lines.append("범위(이 파일/범위만 — 무관 파일·repo 배회 금지): %s" % scope)
    if success:
        lines.append("성공 기준(완료 보고는 이 기준 대비 검증 결과를 포함하라): %s" % success)
    if output_format:
        lines.append("산출 형식(이 형식·구조로 산출하라 — W8 4-part output-format): %s" % output_format)
    lines.append("")
    lines.append("절대 강조 4규칙 (WORKER_DIRECTIVE §3 — 모든 작업에 적용·위반 금지):")
    lines.extend("  " + b for b in bullets)
    lines.append("")
    # 경로는 pack 앵커 절대경로 — javis_report의 todo 스캔 루트(pack/round)와 일치해야
    # 진행% 집계에 잡힌다(상대경로 'round/'는 워커 cwd에 따라 집계 누락 — 적대 검증 R1).
    lines.append("todo 영속: 이 작업을 \"${CYS_PACK_DIR:-$HOME/.cys/pack}/round/%s_TODO.md\"에 "
                 "분해하고 세부 완료마다 체크박스를 갱신하라(진행%% 집계 원천)."
                 % to_role.upper().replace("-", "_"))
    lines.append("보고 채널: 완료·질문·충돌·막힘은 `cys send --queued --to master \"[보고] ...\"` "
                 "로 직접 push하라(--queued는 자동 Return 배달 — send-key 불필요·타이핑 가드 "
                 "안전). 즉시 끼어들어야 할 긴급 보고만 직접 send 후 `cys send-key --to master "
                 "Return`(가드 차단 시 --queued로 전환).")
    return "\n".join(lines)


def cmd_task_prompt(args):
    # 역할명은 kebab-case만 — 오류 메시지·todo 파일명에 그대로 보간되므로 위생 처리(주입 차단).
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", args.to):
        print("[task-prompt] --to 역할명은 kebab-case(a-z0-9-)만 허용: %r" % args.to,
              file=sys.stderr)
        return 2
    # 생존 게이트 (절대지침 5차-1): "워커가 정상 작동하는 것을 확인한 후 작업 지시를 내린다"
    # — 이 확인은 눈대중이 아니라 cys status의 agent_alive로만 확정한다.
    status = cys_status()
    if status is None:
        print("[task-prompt] cys status 수집 실패(데몬 미가동?) — `cys ping` 확인 후 재실행. "
              "대상 생존 미확인 상태로는 티켓을 내지 않는다.", file=sys.stderr)
        return 2
    if not live_roles(status).get(args.to):
        print("[task-prompt] 대상 '%s' 미기동 — 티켓 미출력. `cys boot`(4종 의무 기동) 또는 "
              "`cys launch-agent --role %s --agent claude`로 기동 후 재실행하라."
              % (args.to, args.to), file=sys.stderr)
        return 1
    # '정상 작동' 보조 신호: 장기 idle(기본 5분 — CYS_IDLE_SECONDS와 동기)은 hang일 수
    # 있다 — 차단은 아니고 경고만(지시 대기 중인 워커도 idle이므로 alive가 결정 기준,
    # idle은 §5 능동 점검 트리거). 같은 role의 죽은 stale surface는 건너뛴다.
    try:
        idle_thr = int(os.environ.get("CYS_IDLE_SECONDS", "300"))
    except ValueError:
        idle_thr = 300
    for s in status.get("surfaces", []):
        if s.get("role") == args.to and s.get("agent_alive"):
            idle = s.get("idle_secs")
            if isinstance(idle, (int, float)) and idle >= idle_thr:
                print("[task-prompt] 주의: '%s' idle %d초 — hang 여부를 read-screen으로 "
                      "확인 후 전송하라(§5 능동 점검)." % (args.to, int(idle)), file=sys.stderr)
            break
    rules = extract_worker_rules()
    if rules is None:
        print("[task-prompt] 경고: WORKER_DIRECTIVE '절대 강조 4규칙' 추출 실패 또는 "
              "마커 불완전 — 하드 폴백(FALLBACK_RULES)으로 주입한다. 디렉티브를 점검하라"
              "(preflight C03).", file=sys.stderr)
        rules = FALLBACK_RULES
    print(build_task_ticket(args.task, args.scope, args.success, args.to, rules=rules,
                            output_format=getattr(args, "output_format", None)))
    return 0


# ── phase-plan: Task를 자기완결 Phase 티켓으로 분해 (영상 N6) ──
# 영상: Task=작업 통째, Phase=그 작업을 마치기 위해 나눈 단계들. 각 Phase는 독립 세션이
# "이것만 보고도" 완수하게 자기완결시켜 메인 컨텍스트를 보존한다(rule 인덱스 JSON + 페이지별 지침).
# ★실행은 코드가 `claude -p` raw subprocess를 띄우지 않는다(harness-creator PROMPT_RUNNER_ABSENT
# 철학·자원 거버넌스 충돌 회피) — 스킬이 Phase 티켓을 Workflow pipeline 또는 cys 워커 순차
# 위임으로 실행하도록 안내한다.
def phase_index_path(task):
    safe = re.sub(r"[^0-9A-Za-z가-힣_.-]", "_", task)[:80]
    return os.path.join(pack_dir(), "round", "PHASE-%s.json" % safe)


def cmd_phase_plan(args):
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", args.to):
        print("[phase-plan] --to 역할명은 kebab-case(a-z0-9-)만 허용: %r" % args.to,
              file=sys.stderr)
        return 2
    phases = [p.strip() for p in args.phases.split(";") if p.strip()]
    if not phases:
        print("[phase-plan] --phases가 비었거나 형식 오류(세미콜론 분리 비어있음): %r — "
              "예: --phases \"설계;구현;검증\"" % args.phases, file=sys.stderr)
        return 2
    # 4규칙 주입은 task-prompt와 동일 원천(추출→실패 시 하드 폴백). 위임 게이트(노드 생존)는
    # phase-plan이 즉시 위임하지 않으므로(스킬이 순차 위임) 적용하지 않는다 — 계획 산출 단계.
    rules = extract_worker_rules()
    if rules is None:
        print("[phase-plan] 경고: WORKER_DIRECTIVE '절대 강조 4규칙' 추출 실패 또는 "
              "마커 불완전 — 하드 폴백(전문)으로 강등 주입한다(약화 전파 차단).", file=sys.stderr)
        rules = FALLBACK_RULES
    n = len(phases)
    tickets = []
    index = {"task": args.task, "scope": args.scope, "phases": []}
    for i, name in enumerate(phases, start=1):
        pid = "P%d" % i
        # 각 Phase는 자기완결 — 독립 세션이 이 티켓만 보고도 완수하도록 직전 Phase 산출물·
        # docs-diff 참조를 명시한다(영상: 페이지별 상세 지침·메인 컨텍스트 보존).
        prev = ("직전 Phase(%s) 산출물과 docs-diff(javis_docsdiff.py 변경 줄)를 참조하라."
                % ("P%d" % (i - 1)) if i > 1 else
                "이 작업의 첫 Phase다 — 컨텍스트의 구체화된 계획을 출발점으로 삼는다.")
        phase_task = "[%s/%d] %s — %s" % (pid, n, args.task, name)
        phase_scope = ("%s | 이 Phase만 독립 실행(자기완결): %s. %s 다른 Phase 작업·범위는 "
                       "건드리지 마라." % (args.scope, prev,
                       "산출물은 작업 폴더에 남기고 완료를 master에 push해 다음 Phase를 잇는다."))
        ticket = build_task_ticket(phase_task, phase_scope, args.success, args.to, rules=rules)
        tickets.append((pid, name, ticket))
        index["phases"].append({"id": pid, "name": name, "status": "pending"})
    p = phase_index_path(args.task)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w", encoding="utf-8") as f:
        json.dump(index, f, ensure_ascii=False, indent=2)
    # 사람이 읽는 티켓들(각 Phase 자기완결) + 기계 인덱스 경로를 출력.
    blocks = []
    blocks.append("[phase-plan] Task를 %d개 자기완결 Phase로 분해 — 인덱스: %s" % (n, p))
    blocks.append("실행: 코드는 claude -p를 띄우지 않는다. 아래 Phase 티켓을 Workflow "
                  "pipeline 또는 cys 워커로 순차 위임하라(각 Phase 독립 세션·메인 컨텍스트 보존).")
    for pid, name, ticket in tickets:
        blocks.append("")
        blocks.append("════════ %s · %s ════════" % (pid, name))
        blocks.append(ticket)
    print("\n".join(blocks))
    return 0


# ── round 장부 (결정론 라운드 추적) ──
def round_path(task):
    safe = re.sub(r"[^0-9A-Za-z가-힣_.-]", "_", task)[:80]
    return os.path.join(pack_dir(), "round", "ORCHESTRATION-%s.md" % safe)


def cmd_round_init(args):
    p = round_path(args.task)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    if os.path.exists(p):
        print("이미 존재: %s (round-status로 확인)" % p)
        return 0
    open(p, "w", encoding="utf-8").write(
        "# ORCHESTRATION 라운드 장부 — %s\n\n"
        "> 절대지침 4차 5-1~5-8. 완료조건: 맥킨지급 도달(외부 리뷰어 판정) 또는 %dR 완료.\n"
        "> 자기채점 금지 — score는 producer≠evaluator(외부 리뷰어)가 매긴다.\n\n"
        "| 라운드 | 평가자 | 점수 | 판정 |\n|---|---|---|---|\n" % (args.task, MAX_ROUNDS)
    )
    print("라운드 장부 생성: %s" % p)
    return 0


def _cell(s):
    """Markdown 표 셀 새니타이즈 — 파이프·개행이 표 구조(parse_rounds)를 깨지 않게."""
    return str(s).replace("|", "/").replace("\n", " ").strip()


def cmd_round_log(args):
    p = round_path(args.task)
    if not os.path.exists(p):
        cmd_round_init(args)
    score, verdict = args.score, args.verdict
    machine_fail = False
    # machine 평가자의 결정론 기록(앵커6 축1): --from-cmd는 기계검증 명령을 이 도구가
    # 직접 실행해 exit code로 verdict를 자동 기록한다 — master(전환 이해당사자)의
    # 전사(轉寫)를 거치지 않는 producer≠evaluator 경로.
    if getattr(args, "from_cmd", None):
        try:
            r = subprocess.run(args.from_cmd, shell=True, capture_output=True, timeout=1800)
            verdict = ("PASS(exit 0)" if r.returncode == 0
                       else "FAIL(exit %d)" % r.returncode)
            machine_fail = r.returncode != 0
            tail = (r.stdout or r.stderr or b"").decode("utf-8", "replace").strip()
            score = (tail.splitlines()[-1][:60] if tail else "-")
        except subprocess.TimeoutExpired:
            verdict, score, machine_fail = "FAIL(timeout 1800s)", "-", True
    elif evaluator_std(args.evaluator) == "machine":
        # 전사 금지(앵커6 축1·MASTER §14): machine 행은 --from-cmd 자동 기록이 규약 —
        # 수기 verdict는 기록하되 경고를 남긴다(기존 호환 유지·게이트 신뢰는 운영 규약).
        print("[round-log] 경고: machine 평가자를 --from-cmd 없이 수기 기록 중 — "
              "전사 금지 규약(MASTER §14) 위반 소지. --from-cmd \"<명령>\"을 써라.",
              file=sys.stderr)
    with open(p, "a", encoding="utf-8") as f:
        f.write("| %d | %s | %s | %s |\n"
                % (args.round, _cell(args.evaluator), _cell(score), _cell(verdict)))
    print("기록: 라운드 %d · 평가자 %s · 점수 %s · 판정 %s"
          % (args.round, _cell(args.evaluator), _cell(score), _cell(verdict)))
    # --from-cmd 검증 실패는 exit 1 — 기록은 성공했지만 && 체인이 "검증 통과"로
    # 오독하지 않게 한다(판정의 단일 진실은 gate-status).
    return 1 if machine_fail else 0


def parse_rounds(p):
    rows = []
    try:
        for ln in open(p, encoding="utf-8"):
            m = re.match(r"\|\s*(\d+)\s*\|\s*([^|]*?)\s*\|\s*([^|]*?)\s*\|\s*([^|]*?)\s*\|", ln)
            if m:
                rows.append({"round": int(m.group(1)), "evaluator": m.group(2),
                             "score": m.group(3), "verdict": m.group(4)})
    except OSError:
        return []
    return rows


def cmd_round_status(args):
    p = round_path(args.task)
    if not os.path.exists(p):
        print("라운드 장부 없음: %s — `round-init`로 생성" % p)
        return 1
    rows = parse_rounds(p)
    last = max((r["round"] for r in rows), default=0)
    print("라운드 현황 — %s" % args.task)
    print("  기록된 라운드: %d / 상한 %d" % (last, MAX_ROUNDS))
    if rows:
        r = rows[-1]
        print("  최근: 라운드 %d · 평가자 %s · 점수 %s · 판정 %s"
              % (r["round"], r["evaluator"], r["score"], r["verdict"]))
    if last >= MAX_ROUNDS:
        print("  → %dR 상한 도달: 무한 루프 금지. 맥킨지급 미달이면 오너에게 격차 보고하라." % MAX_ROUNDS)
        return 0
    print("  → 다음 라운드 %d 진행 가능(맥킨지급 도달 전까지). 외부 리뷰어가 +10%% 목표로 평가." % (last + 1))
    return 0


# ── 자율주행 위임권 (앵커6) — 축1 게이트 4자 수렴 · 축3 다음 액션 큐 ──
# 축1: "게이트 4자 수렴(gemini+codex+master+기계검증)+커밋+SESSION_STATE 갱신 = 다음 단계
# 자동 착수". 수렴 여부를 LLM 눈대중이 아니라 round 장부의 기록으로만 판정한다.
GATE_EVALUATORS = ("gemini", "codex", "master", "machine")
# 표기 이주(2026-06-13): 구 Gemini CLI → Antigravity CLI(agy). 문서·라운드 기록이 'agy'로
# 표기해도 표준 평가자 'gemini'로 매핑한다(역할명 reviewer-gemini·어댑터 키 'gemini'와의
# 계약은 무변 — 식별자 층은 유지, 표기 층만 agy).
EVALUATOR_ALIASES = {"agy": "gemini"}
APPROVE_PREFIXES = ("pass", "수렴", "approve", "ok", "green", "승인")
# 부정 토큰이 하나라도 있으면 무조건 미승인 — 한국어 부정은 접미에 붙으므로("승인 불가"·
# "수렴 실패") 접두 매칭만으로는 게이트가 열린다(적대 검증 6차 R1 HIGH-1). 부정이 승인을
# 이긴다(안전 우선: 모호하면 닫힘).
REJECT_MARKERS = ("실패", "불가", "반려", "미달", "거부", "아님", "보류", "미흡", "미승인",
                  "fail", "reject", "deny", "denied", "no-go", "block", "not ")


def verdict_approved(verdict):
    """verdict 문자열의 승인 판정 — 부정 토큰 우선 차단, 그 다음 승인 접두(순수 함수)."""
    v = verdict.strip().lower()
    if any(m in v for m in REJECT_MARKERS):
        return False
    return any(v.startswith(p) for p in APPROVE_PREFIXES)


def evaluator_std(evaluator):
    """평가자 문자열 → 표준 평가자. 정확 일치 또는 구분자(:·-·공백) 접두만 인정 —
    'masterful'·'machinelearning' 류 오탐 차단(적대 검증 6차 R1 LOW-7).
    별칭(agy→gemini)도 같은 규칙으로 수용한다."""
    ev = evaluator.strip().lower()
    candidates = [(e, e) for e in GATE_EVALUATORS] + list(EVALUATOR_ALIASES.items())
    for name, std in candidates:
        if ev == name or ev.startswith(name + ":") or ev.startswith(name + "-") \
                or ev.startswith(name + " "):
            return std
    return None


def gate_verdicts(rows, rnd):
    """라운드 rnd의 평가자별 최종 verdict 승인 여부 — 순수 함수(self-test 박제).

    같은 평가자가 같은 라운드에 여러 번 기록하면 마지막 기록이 이긴다(재평가 허용).
    반환: {표준 평가자: bool|None}.
    """
    out = {e: None for e in GATE_EVALUATORS}
    for r in rows:
        if r["round"] != rnd:
            continue
        std = evaluator_std(r["evaluator"])
        if std:
            out[std] = verdict_approved(r["verdict"])
    return out


def cmd_gate_status(args):
    p = round_path(args.task)
    if not os.path.exists(p):
        print("[gate-status] 라운드 장부 없음: %s — round-init·round-log로 기록을 쌓아라"
              % p, file=sys.stderr)
        return 1
    rows = parse_rounds(p)
    rnd = args.round or max((r["round"] for r in rows), default=0)
    if rnd <= 0:
        print("[gate-status] 기록된 라운드 없음 — 미수렴", file=sys.stderr)
        return 1
    verdicts = gate_verdicts(rows, rnd)
    missing = [e for e, v in verdicts.items() if v is None]
    rejected = [e for e, v in verdicts.items() if v is False]
    print("게이트 4자 수렴 판정 — %s (라운드 %d)" % (args.task, rnd))
    for e in GATE_EVALUATORS:
        v = verdicts[e]
        print("  %s %s — %s" % ("✓" if v else "✗", e,
                                "승인" if v else ("기록 없음" if v is None else "미승인")))
    if missing or rejected:
        print("종합: 미수렴 — %s%s. 자동 착수 불가(라운드 계속 또는 오너 보고)."
              % (("누락: " + ", ".join(missing)) if missing else "",
                 ((" / " if missing else "") + "미승인: " + ", ".join(rejected))
                 if rejected else ""))
        return 1
    # 보조 결정론(차단 아님): SESSION_STATE가 장부 마지막 기록보다 오래됐으면 "갱신" 요건
    # 미이행 가능성 경고 — 갱신은 전환 직전 수행이 규약이므로 순서상 이후일 수 있어 경고만.
    ss = os.path.join(pack_dir(), "round", "SESSION_STATE.md")
    try:
        if os.path.getmtime(ss) < os.path.getmtime(p):
            print("[gate-status] 주의: SESSION_STATE.md가 라운드 장부보다 오래됨 — 전환 전 "
                  "갱신 요건(축1)을 이행했는지 확인하라.", file=sys.stderr)
    except OSError:
        pass
    print("종합: GATE CONVERGED — 4자 수렴. 커밋+SESSION_STATE 갱신 후 다음 로드맵 단계를 "
          "자동 착수하라(앵커6 축1 — denylist 해당 시에만 정지).")
    return 0


def extract_next_action(text):
    """SESSION_STATE '## 다음 액션' 섹션의 첫 미완 항목 — 순수 함수(self-test 박제).

    지원 형식: 'N. 항목' 번호 목록 · '- [ ] 항목' 체크박스 · '- 항목' 불릿.
    제외: '(없음)' 류 빈 표시 · 완료 체크(- [x]). 반환: 항목 문자열 또는 None.
    """
    m = re.search(r"(?m)^##\s*다음 액션[^\n]*\n(.*?)(?:\n##\s|\Z)", text, re.S)
    if not m:
        return None
    for ln in m.group(1).splitlines():
        s = ln.strip()
        if not s:
            continue
        item = None
        nm = re.match(r"^\d+\.\s+(.*)$", s)
        if nm:
            item = nm.group(1).strip()
        elif s.startswith("- "):
            item = s[2:].strip()
        if item is None:
            continue
        # 번호·불릿 공통: 체크박스 완료([x])는 건너뛰고 미완([ ])은 마커를 벗긴다 —
        # "1. [x] 끝난 일"이 다음 액션으로 반환되면 완료 작업 재실행 루프가 된다(6차 R1).
        if item.lower().startswith("[x]"):
            continue
        if item.startswith("[ ]"):
            item = item[3:].strip()
        # 빈 표시: '없음' 단독 또는 괄호/구두점 부가 설명만 빈 칸이다 — "없음 처리 로직
        # 구현" 같은 실제 과제명은 빈 칸이 아니다(시작-매칭 과확장 차단, 6차 R2).
        if item and not re.match(r"^[\(（]?\s*없음\s*[\)）.。\s]*([\(（].*)?$", item):
            return item
    return None


def cmd_next_action(args):
    # exit 계약: 0=다음 액션 있음(stdout) / 1=빈 큐(전 작업 완료 — 정지·오너 보고) /
    # 2=SESSION_STATE 부재(신규 시작 — 오너 지시 대기). 1과 2는 다른 대응이다(§0-⑥ vs §14).
    p = os.path.join(pack_dir(), "round", "SESSION_STATE.md")
    try:
        text = open(p, encoding="utf-8", errors="replace").read()
    except OSError:
        print("[next-action] SESSION_STATE 없음(신규 시작): %s — 오너 지시를 기다려라."
              % p, file=sys.stderr)
        return 2
    item = extract_next_action(text)
    if item is None:
        print("[next-action] 다음 액션 큐 비어 있음 — 전 작업 완료. 자율 루프 정지·오너 보고.",
              file=sys.stderr)
        return 1
    print(item)
    return 0


def cmd_self_test(args):
    """순수 로직 자기검증 (cys 의존 없음) — preflight C19가 호출. assert 실패는 exit 1."""
    try:
        assert REQUIRED_ROLES == ["cso", "worker", "reviewer-gemini", "reviewer-codex"], \
            "4종 의무 노드 목록이 변형됐다"
        assert MAX_ROUNDS == 10, "라운드 상한은 10이어야 한다(앵커4 5-8)"
        # round_path 경로 탈출 방지: 악성 task가 round 디렉터리 밖으로 못 나간다(실효 검증).
        rnd_dir = os.path.realpath(os.path.join(pack_dir(), "round"))
        for evil in ("../../etc/passwd", "a/b ../x:일", "..\\..\\win", "/abs/x"):
            ep = os.path.realpath(os.path.dirname(round_path(evil)))
            assert ep == rnd_dir, "round_path 경로 탈출: %s → %s" % (evil, ep)
            assert os.sep not in os.path.basename(round_path(evil)).replace(
                "ORCHESTRATION-", "").replace(".md", "").replace("_", ""), "basename 분리자 잔존"
        # review-prompt 생성: 제약·형식이 항상 포함된다(폴백 포함)
        class _A:
            task, scope, reviewer, round = "T", "S", None, 2
        import io
        import contextlib
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            cmd_review_prompt(_A())
        out = buf.getvalue()
        for must in ("엄격 제약", "배회 금지", "문제점", "회신", "+10%"):
            assert must in out, "review-prompt에 '%s' 누락" % must
        # live_roles 파싱
        lr = live_roles({"surfaces": [
            {"role": "cso", "agent_alive": True},
            {"role": "worker", "agent_alive": False},
        ]})
        assert lr == {"cso": True}, "live_roles 파싱 오류"
        # round-log 표 셀 새니타이즈: 파이프·개행이 제거된다
        assert _cell("a|b\nc") == "a/b c", "_cell 새니타이즈 오류"
        # task-prompt 티켓(밀폐 — rules 명시 주입, 설치본 디렉티브 상태와 무관):
        # 절대 강조 4규칙·게이트·todo(pack 앵커)·보고 채널이 항상 포함된다
        ticket = build_task_ticket("T", "S", "C", "worker", rules=FALLBACK_RULES)
        for must in ("절대 강조 4규칙", "품질 절대우선", "할루시네이션 방지",
                     "hallucination-guard", "grill-me", "요약·압축 절대 금지", "게이트",
                     "성공 기준", "WORKER_TODO.md", "${CYS_PACK_DIR", "보고 채널",
                     "--queued"):
            assert must in ticket, "task-prompt 티켓에 '%s' 누락" % must
        # 폴백 단독으로도 4규칙 마커 전부를 갖는다(디렉티브 부재 환경의 최후 방어선)
        fb = "\n".join(FALLBACK_RULES)
        for mark in RULE_MARKERS:
            assert mark in fb, "FALLBACK_RULES에 마커 '%s' 누락" % mark
        # --success 생략 시 성공 기준 라인이 사라진다(빈 값 주입 금지)
        assert "성공 기준" not in build_task_ticket("T", "S", None, "worker",
                                                  rules=FALLBACK_RULES), \
            "success 미지정인데 성공 기준 라인 존재"
        # todo 파일명은 역할명 대문자 변환(reviewer-gemini → REVIEWER_GEMINI_TODO.md)
        assert "REVIEWER_GEMINI_TODO.md" in build_task_ticket(
            "T", "S", None, "reviewer-gemini", rules=FALLBACK_RULES), "todo 파일명 역할 변환 오류"
        # 추출기(순수 함수) 배터리 — 합성 디렉티브 텍스트로 밀폐 검증:
        synth = ("# W\n\n## 7. ★절대 강조 4규칙 — x\n머리말.\n"
                 + "\n".join(FALLBACK_RULES) + "\n\n## 8. 다음\n- 무관\n")
        got = extract_rules_from_text(synth)
        assert got and len(got) == len(FALLBACK_RULES), "추출 개수 불일치(머리말 혼입?)"
        # (a) 절 번호가 3이 아니어도 추출된다(번호 하드코딩 금지)
        # (b) 멀티라인 wrap: 불릿을 두 줄로 쪼개도 연속줄 합류로 마커가 보존된다
        wrapped = synth.replace("몽상·망상을 촉진하는 말 절대 금지.",
                                "\n  몽상·망상을 촉진하는 말 절대 금지.")
        gw = extract_rules_from_text(wrapped)
        assert gw and "몽상" in "\n".join(gw), "연속줄 합류 실패 — wrap 잘림"
        # (c) 약화된 디렉티브(마커 소실)는 추출 거부 → 폴백 강등(전파 차단)
        assert extract_rules_from_text(synth.replace("Garbage-in", "")) is None, \
            "약화 디렉티브가 추출을 통과(전파 위험)"
        # (d) 섹션 부재 → None
        assert extract_rules_from_text("# 없음\n## 1. 다른 절\n- x\n") is None, \
            "무관 텍스트에서 추출 오탐"
        # 자율주행(앵커6) — gate_verdicts 순수 배터리: 4자 수렴/누락/미승인/재평가 우선
        rows = [{"round": 1, "evaluator": "gemini", "score": "9", "verdict": "PASS 95"},
                {"round": 1, "evaluator": "codex-r1", "score": "9", "verdict": "수렴"},
                {"round": 1, "evaluator": "master", "score": "-", "verdict": "approve"},
                {"round": 1, "evaluator": "machine:cargo", "score": "159", "verdict": "green"}]
        assert all(gate_verdicts(rows, 1).values()), "4자 전원 승인인데 미수렴 판정"
        assert gate_verdicts(rows[:3], 1)["machine"] is None, "machine 누락 미검출"
        rows2 = rows + [{"round": 1, "evaluator": "codex", "score": "5", "verdict": "반려"}]
        assert gate_verdicts(rows2, 1)["codex"] is False, "재평가(마지막 기록 우선) 미반영"
        assert gate_verdicts(rows, 2) == {e: None for e in GATE_EVALUATORS}, \
            "다른 라운드 기록이 새 라운드에 새는 오염"
        # ★부정 verdict 차단(6차 R1 HIGH-1): 한국어 부정 접미·영문 부정이 승인으로 새면
        # 가짜 GATE CONVERGED로 자율 전진한다 — 전부 False여야 한다.
        for neg in ("수렴 실패", "수렴 미달", "승인 불가", "승인 보류", "승인 거부",
                    "ok지만 반려", "pass 불가", "green 아님", "approve 거부", "PASS fail",
                    "ok — not yet", "미승인"):
            assert verdict_approved(neg) is False, "부정 verdict '%s'가 승인 오판" % neg
        for pos in ("PASS 95점", "수렴", "approve", "green", "승인."):
            assert verdict_approved(pos) is True, "정상 승인 '%s'가 거부 오판" % pos
        # ★평가자 구분자 강제(6차 R1 LOW-7): 가짜 접두는 무시, 구분자 변형은 인정
        assert evaluator_std("masterful-bot") is None, "'masterful' 오탐"
        assert evaluator_std("machinelearning") is None, "'machinelearning' 오탐"
        assert evaluator_std("machine:pytest") == "machine" and \
            evaluator_std("codex-r1") == "codex" and evaluator_std("gemini") == "gemini", \
            "정상 평가자 변형 매칭 실패"
        # 표기 이주 별칭: agy(Antigravity CLI) 기록도 표준 gemini로 — 'agycorp' 류는 거부
        assert evaluator_std("agy") == "gemini" and evaluator_std("agy:r2") == "gemini", \
            "agy 별칭 매핑 실패"
        assert evaluator_std("agycorp") is None, "'agycorp' 오탐"
        # 자율주행(앵커6) — extract_next_action 순수 배터리
        ss = ("# S\n## 다음 액션 큐\n1. (없음)\n\n## 기타\n- x\n")
        assert extract_next_action(ss) is None, "'(없음)' 빈 큐 오탐"
        ss2 = "# S\n## 다음 액션 큐\n1. 6차 블록 검증\n2. 다음\n"
        assert extract_next_action(ss2) == "6차 블록 검증", "번호 목록 첫 항목 추출 실패"
        ss3 = "# S\n## 다음 액션\n- [x] 끝난 일\n- [ ] 남은 일\n"
        assert extract_next_action(ss3) == "남은 일", "체크박스 미완 항목 추출 실패"
        assert extract_next_action("# S\n## 다른 절\n- x\n") is None, "섹션 부재 오탐"
        # ★번호+체크박스 혼용(6차 R1 MED-3): 완료([x])는 건너뛰고 미완([ ])은 마커 제거
        ss4 = "# S\n## 다음 액션 큐\n1. [x] 끝난 일\n2. [ ] 남은 일\n"
        assert extract_next_action(ss4) == "남은 일", "번호+[x] 완료 항목이 액션으로 반환"
        # ★'없음' 변형(6차 R1): 전각 괄호·부가 설명도 빈 칸이다
        for empty in ("1. （없음）\n", "1. 없음 (전 작업 완료)\n", "- (없음).\n"):
            assert extract_next_action("# S\n## 다음 액션 큐\n" + empty) is None, \
                "'없음' 변형 '%s'가 액션으로 반환" % empty.strip()
        # ★'없음' 시작-매칭 과확장 차단(6차 R2): "없음 처리 로직" 같은 실제 과제는 빈 칸 아님
        assert extract_next_action("# S\n## 다음 액션 큐\n1. 없음 처리 로직 구현\n") \
            == "없음 처리 로직 구현", "'없음'으로 시작하는 실제 과제가 silent skip"
        # (e) 핀↔마커 패리티: 마커 소실로 폴백 강등될 때 안내하는 preflight C03(WORKER 핀)이
        # 같은 소실을 검출할 수 있어야 진단 루프가 닫힌다. javis_preflight가 같은 bin에
        # 있을 때만 검사(없는 환경에서는 자기 검증 불가 — 건너뜀).
        pf_path = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                               "javis_preflight.py")
        if os.path.isfile(pf_path):
            import importlib.util
            spec = importlib.util.spec_from_file_location("_pf_parity", pf_path)
            _pf = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(_pf)
            worker_pins = [p for p, _ in _pf.CONTENT_PINS.get("WORKER_DIRECTIVE.md", [])]
            for mark in RULE_MARKERS:
                assert any(mark in pin or pin in mark for pin in worker_pins), \
                    "마커 '%s'가 WORKER C03 핀에 비커버 — 폴백 강등 원인을 preflight가 못 본다" % mark
    except AssertionError as e:
        print("javis_orchestra self-test FAIL: %s" % e, file=sys.stderr)
        return 1
    print("javis_orchestra self-test OK (4종 노드·라운드 상한·경로 탈출방지·제약 주입·"
          "4규칙 티켓 주입·파싱·셀 새니타이즈)")
    return 0


def main():
    # preflight 호환: `--self-test`는 subcommand 없이도 동작해야 한다(가로채기).
    if "--self-test" in sys.argv:
        return cmd_self_test(None)
    ap = argparse.ArgumentParser(description="LLM 오케스트레이션 결정론 도구(앵커4)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    sub.add_parser("check", help="4종 의무 노드 생존 판정")

    rp = sub.add_parser("review-prompt", help="제약 포함 리뷰 의뢰 프롬프트 생성")
    rp.add_argument("--task", required=True)
    rp.add_argument("--scope", required=True, help="검토 대상 파일/범위")
    rp.add_argument("--reviewer", choices=["gemini", "codex"], default=None)
    rp.add_argument("--round", type=int, default=1)
    rp.add_argument("--success", default=None,
                    help="평가 기준(구현 위임과 동일 — 리뷰어에게도 같은 기준 투입, N3 양방향)")

    tp = sub.add_parser("task-prompt", help="생존 게이트 + 절대 강조 4규칙 포함 위임 티켓 생성")
    tp.add_argument("--task", required=True)
    tp.add_argument("--scope", required=True, help="작업 대상 파일/범위")
    tp.add_argument("--success", default=None, help="성공 기준 (완료 보고의 검증 기준)")
    tp.add_argument("--to", default="worker", help="위임 대상 역할 (기본 worker)")
    tp.add_argument("--output-format", default=None,
                    help="산출 형식·구조 (W8 4-part output-format 슬롯 — 예: 'JSON {필드}', '마크다운 표', '보고서 PDF')")

    pp = sub.add_parser("phase-plan",
                        help="Task를 자기완결 Phase 티켓으로 분해 (영상 N6 — Task/Phase 순차)")
    pp.add_argument("--task", required=True)
    pp.add_argument("--phases", required=True, help="세미콜론 분리 Phase 이름들 (예: \"설계;구현;검증\")")
    pp.add_argument("--scope", required=True, help="작업 대상 파일/범위")
    pp.add_argument("--success", default=None, help="성공 기준 (각 Phase 티켓에 동일 투입)")
    pp.add_argument("--to", default="worker", help="위임 대상 역할 (기본 worker)")

    ri = sub.add_parser("round-init"); ri.add_argument("--task", required=True)
    rl = sub.add_parser("round-log")
    rl.add_argument("--task", required=True); rl.add_argument("--round", type=int, required=True)
    rl.add_argument("--evaluator", required=True); rl.add_argument("--score", default="-")
    rl.add_argument("--verdict", default="")
    rl.add_argument("--from-cmd", dest="from_cmd", default=None,
                    help="기계검증 명령을 직접 실행해 exit code로 verdict 자동 기록"
                         "(machine 평가자 권장 — 전사 없는 producer≠evaluator 경로)")
    rs = sub.add_parser("round-status"); rs.add_argument("--task", required=True)

    gs = sub.add_parser("gate-status", help="자율주행 축1 — 4자 수렴 결정론 판정")
    gs.add_argument("--task", required=True)
    gs.add_argument("--round", type=int, default=None, help="생략 시 최신 라운드")

    sub.add_parser("next-action", help="자율주행 축3 — SESSION_STATE 다음 액션 큐 첫 미완 항목")

    args = ap.parse_args()
    return {
        "check": cmd_check,
        "review-prompt": cmd_review_prompt,
        "task-prompt": cmd_task_prompt,
        "phase-plan": cmd_phase_plan,
        "round-init": cmd_round_init,
        "round-log": cmd_round_log,
        "round-status": cmd_round_status,
        "gate-status": cmd_gate_status,
        "next-action": cmd_next_action,
    }[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
