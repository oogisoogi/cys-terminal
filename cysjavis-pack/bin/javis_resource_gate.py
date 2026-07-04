#!/usr/bin/env python3
"""javis_resource_gate.py — P0-3 자원 사전 게이트 (getInvocationBlock의 정액제 번안)

계약(출처: _research/Paperclip_박사급_연구보고서.md §4 P0-3 · §2-7):
- Paperclip의 진짜 런어웨이 차단 = "새 run 시작 전 라이브 재계산해 초과면 착수 거부"(사전 게이트).
- 정액제(Claude Max)엔 달러 예산이 무력하므로 metric을 자원으로 치환:
    servers  = 로컬 dev/서버 프로세스 수         (자원 거버넌스 '서버 누적' 사고 이력)
    nodes    = claude/agy/codex 노드 프로세스 수
    load     = 1분 load average / CPU 코어 수 비율
    context  = 자기보고 컨텍스트 %               (60% /clear 규칙)
- soft/hard 2단(Paperclip warnPercent 사상): soft=경고 후 진행 허용, hard=착수 거부.
- 판정은 결정론: exit code 0=allow · 1=soft warn · 2=hard block. (LLM 자연어 판단 제거)
- "저장값 재신뢰 금지, 매번 재계산" — 게이트는 항상 라이브 측정.

기본 임계(우리 자원 거버넌스 실사고 기준):
  servers  soft 2  / hard 3     (watchdog '3개+' 규칙과 정합 — 사후 kill 전에 사전 차단)
  nodes    soft 8  / hard 12
  load     soft 1.0×ncpu / hard 2.0×ncpu
  context  soft 50 / hard 60    (60% 도달 전 저장 후 /clear 규칙)

테스트/자동화 주입: --servers-override/--nodes-override/--load-override (라이브 측정 대체).
사용 예: python3 javis_resource_gate.py check --context 42 --json
exit codes: 0 allow · 1 soft · 2 hard · (3+ 내부 오류)
"""
import argparse
import json
import os
import re
import subprocess
import sys

EXIT_ALLOW, EXIT_SOFT, EXIT_HARD = 0, 1, 2

SERVER_PATTERNS = [
    r"bun .*server", r"node .*server", r"vite(\s|$)", r"next dev", r"uvicorn",
    r"python3? -m http\.server", r"python3? .*server\.py", r"webpack.*serve",
]
# 서버가 아닌 상주 인프라(오탐 제외): 언어 서버(LSP)·MCP 서버 등은 자원 거버넌스의
# 'dev 서버 누적' 대상이 아니다 (실측: pyright langserver.index.js가 node .*server에 걸림).
SERVER_EXCLUDE_PATTERNS = [
    r"langserver", r"language[-_ ]?server", r"\blsp\b", r"mcp[-_ ]?server",
    r"tsserver", r"copilot",
]
NODE_PATTERNS = [r"claude(\s|$)", r"\bagy\b", r"\bcodex\b", r"\bgemini\b"]


def _ps_lines():
    try:
        out = subprocess.run(["ps", "-axo", "pid,command"], capture_output=True,
                             text=True, timeout=10).stdout
        return out.splitlines()[1:]
    except (subprocess.SubprocessError, OSError):
        return []


def _count_matching(lines, patterns, exclude_patterns=()):
    regs = [re.compile(p) for p in patterns]
    excl = [re.compile(p, re.IGNORECASE) for p in exclude_patterns]
    n = 0
    for line in lines:
        cmd = line.strip().split(None, 1)[-1] if line.strip() else ""
        if "javis_resource_gate" in cmd:
            continue
        if any(r.search(cmd) for r in regs) and not any(r.search(cmd) for r in excl):
            n += 1
    return n


def measure(a):
    lines = None
    if a.servers_override is not None:
        servers = a.servers_override
    else:
        lines = _ps_lines()
        servers = _count_matching(lines, SERVER_PATTERNS, SERVER_EXCLUDE_PATTERNS)
    if a.nodes_override is not None:
        nodes = a.nodes_override
    else:
        if lines is None:
            lines = _ps_lines()
        nodes = _count_matching(lines, NODE_PATTERNS)
    if a.load_override is not None:
        load1 = a.load_override
    else:
        try:
            load1 = os.getloadavg()[0]
        except OSError:
            load1 = 0.0
    ncpu = os.cpu_count() or 1
    return {"servers": servers, "nodes": nodes, "load1": round(load1, 2),
            "ncpu": ncpu, "load_ratio": round(load1 / ncpu, 3),
            "context_pct": a.context}


def evaluate(m, a):
    checks = []

    def add(metric, value, soft, hard):
        if value is None:
            return
        level = "hard" if value >= hard else ("soft" if value >= soft else "ok")
        checks.append({"metric": metric, "value": value, "soft": soft,
                       "hard": hard, "level": level})

    add("servers", m["servers"], a.servers_soft, a.servers_hard)
    add("nodes", m["nodes"], a.nodes_soft, a.nodes_hard)
    add("load_ratio", m["load_ratio"], a.load_soft_ratio, a.load_hard_ratio)
    add("context_pct", m["context_pct"], a.context_soft, a.context_hard)

    worst = "ok"
    for c in checks:
        if c["level"] == "hard":
            worst = "hard"
            break
        if c["level"] == "soft":
            worst = "soft"
    return worst, checks


def cmd_check(a):
    m = measure(a)
    worst, checks = evaluate(m, a)
    verdict = {"ok": "allow", "soft": "soft_warn", "hard": "hard_block"}[worst]
    trips = [c for c in checks if c["level"] != "ok"]
    result = {"verdict": verdict, "measured": m, "trips": trips, "checks": checks}
    if a.json:
        print(json.dumps(result, ensure_ascii=False, indent=1))
    else:
        print(f"verdict: {verdict}")
        for c in checks:
            mark = {"ok": "·", "soft": "⚠", "hard": "✗"}[c["level"]]
            print(f"  {mark} {c['metric']}={c['value']} (soft {c['soft']} / hard {c['hard']})")
        if worst == "hard":
            print("hard_block: 착수 거부 — 자원 정리(서버 kill·/clear·노드 회수) 후 재시도하거나 "
                  "master 승인으로 임계 상향. (사후 watchdog와 별개의 사전 게이트)")
        elif worst == "soft":
            print("soft_warn: 진행 허용하되 경고 push 권장.")
    return {"ok": EXIT_ALLOW, "soft": EXIT_SOFT, "hard": EXIT_HARD}[worst]


def cmd_classify(a):
    """stdin의 ps 형식 줄들을 패턴으로 분류(테스트·디버그용 결정론 경로)."""
    lines = sys.stdin.read().splitlines()
    result = {
        "servers": _count_matching(lines, SERVER_PATTERNS, SERVER_EXCLUDE_PATTERNS),
        "nodes": _count_matching(lines, NODE_PATTERNS),
    }
    print(json.dumps(result, ensure_ascii=False))
    return EXIT_ALLOW


def main(argv=None):
    p = argparse.ArgumentParser(description="자원 사전 게이트 — 착수 전 차단 (P0-3)")
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("check")
    c.add_argument("--context", type=float, default=None, help="자기보고 컨텍스트 %%")
    c.add_argument("--json", action="store_true")
    c.add_argument("--servers-soft", type=int, default=2)
    c.add_argument("--servers-hard", type=int, default=3)
    c.add_argument("--nodes-soft", type=int, default=8)
    c.add_argument("--nodes-hard", type=int, default=12)
    c.add_argument("--load-soft-ratio", type=float, default=1.0)
    c.add_argument("--load-hard-ratio", type=float, default=2.0)
    c.add_argument("--context-soft", type=float, default=50.0)
    c.add_argument("--context-hard", type=float, default=60.0)
    c.add_argument("--servers-override", type=int, default=None, help="테스트 주입")
    c.add_argument("--nodes-override", type=int, default=None, help="테스트 주입")
    c.add_argument("--load-override", type=float, default=None, help="테스트 주입")
    c.set_defaults(fn=cmd_check)

    c = sub.add_parser("classify")
    c.set_defaults(fn=cmd_classify)

    a = p.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
