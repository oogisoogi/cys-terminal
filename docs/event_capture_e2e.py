#!/usr/bin/env python3
"""T7 E1-④ 툴 이벤트 캡처 E2E — hook→CLI→usage.event RPC→events 테이블 적재를 실측한다.

검증:
  (1) `cys usage-event-stdin`(cys-hook.sh가 호출하는 plumbing)에 PreToolUse Skill JSON을 물리면
      events 테이블에 is_skill=1·skill_name 파생까지 적재.
  (2) PostToolUse Bash(tool_response.is_error=true) → exit_code=1 적재(E3 반복실패 토대).
  (3) Task 툴 PreToolUse → is_agent=1·agent_type 파생.
  (4) 관심 없는 hook(Notification) → 무적재(fail-open·무차단).
  (5) ★불변: CLI는 항상 exit 0(에이전트 무차단).

실행: cargo build --bins && python3 docs/event_capture_e2e.py
"""
import json
import os
import socket
import sqlite3
import subprocess
import time

DIR = f"/tmp/cys-e14-{os.getpid()}"
os.makedirs(DIR, exist_ok=True)
SOCK = os.path.join(DIR, "cys.sock")
ROOT = os.path.join(os.path.dirname(__file__), "..")
CYSD = os.path.join(ROOT, "target", "debug", "cysd")
CYS = os.path.join(ROOT, "target", "debug", "cys")
ENV = dict(os.environ, CYS_SOCKET=SOCK, CYS_PACK_DIR=os.path.join(DIR, "pack"))
FAIL = []


def check(name, cond, detail=""):
    print(f"[{'PASS' if cond else 'FAIL'}] {name}" + (f" — {detail}" if detail and not cond else ""))
    if not cond:
        FAIL.append(name)


def rpc(method, params):
    s = socket.socket(socket.AF_UNIX)
    s.connect(SOCK)
    s.sendall((json.dumps({"id": 1, "method": method, "params": params}) + "\n").encode())
    b = b""
    while not b.endswith(b"\n"):
        c = s.recv(65536)
        if not c:
            break
        b += c
    s.close()
    return json.loads(b)


def start_daemon():
    p = subprocess.Popen([CYSD], env=ENV, stdout=open(os.path.join(DIR, "cysd.log"), "a"), stderr=subprocess.STDOUT)
    for _ in range(50):
        try:
            if rpc("system.ping", {}):
                return p
        except OSError:
            time.sleep(0.1)
    return p


def fire_hook(sid, payload):
    """cys-hook.sh가 하듯 hook JSON을 stdin으로 cys usage-event-stdin에 물린다. exit code 반환."""
    env = dict(ENV, CYS_SURFACE_ID=str(sid))
    r = subprocess.run([CYS, "usage-event-stdin"], env=env,
                       input=json.dumps(payload).encode(),
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return r.returncode


def events(dbp):
    return sqlite3.connect(dbp).execute(
        "SELECT event_type, tool_name, is_skill, skill_name, is_agent, agent_type, exit_code "
        "FROM events ORDER BY id").fetchall()


def main():
    daemon = start_daemon()
    try:
        sid = rpc("surface.create", {"cmd": "sleep 600", "title": "e14", "cwd": "/tmp"})["result"]["surface_id"]
        rpc("surface.set_meta", {"surface_id": sid, "agent": "claude", "agent_bin": "claude"})

        ec1 = fire_hook(sid, {"hook_event_name": "PreToolUse", "session_id": "s-e14",
                              "tool_name": "Skill", "tool_input": {"skill": "/commit"}})
        ec2 = fire_hook(sid, {"hook_event_name": "PostToolUse", "session_id": "s-e14",
                              "tool_name": "Bash", "tool_response": {"is_error": True}})
        ec3 = fire_hook(sid, {"hook_event_name": "PreToolUse", "session_id": "s-e14",
                              "tool_name": "Task", "tool_input": {"subagent_type": "Explore"}})
        ec4 = fire_hook(sid, {"hook_event_name": "Notification", "message": "noise"})
        check("CLI 항상 exit 0 (무차단 불변)", ec1 == 0 and ec2 == 0 and ec3 == 0 and ec4 == 0,
              f"codes={ec1},{ec2},{ec3},{ec4}")
        time.sleep(0.4)  # RPC 적재 여유

        dbp = os.path.join(DIR, "analytics.db")
        check("analytics.db 생성", os.path.exists(dbp))
        rows = events(dbp) if os.path.exists(dbp) else []
        check("events 3건 적재(관심 hook만 · Notification 무시)", len(rows) == 3, f"rows={rows}")

        skill = [r for r in rows if r[1] == "Skill"]
        check("Skill → is_skill=1·skill_name='commit'(슬래시 strip)",
              len(skill) == 1 and skill[0][2] == 1 and skill[0][3] == "commit", str(skill))
        check("Skill event_type=PRE_TOOL", len(skill) == 1 and skill[0][0] == "PRE_TOOL", str(skill))

        bash = [r for r in rows if r[1] == "Bash"]
        check("PostToolUse Bash is_error → exit_code=1",
              len(bash) == 1 and bash[0][0] == "POST_TOOL" and bash[0][6] == 1, str(bash))

        task = [r for r in rows if r[1] == "Task"]
        check("Task → is_agent=1·agent_type='Explore'",
              len(task) == 1 and task[0][4] == 1 and task[0][5] == "Explore", str(task))
    finally:
        daemon.terminate()
        try:
            daemon.wait(timeout=5)
        except subprocess.TimeoutExpired:
            daemon.kill()

    print()
    if FAIL:
        print(f"❌ {len(FAIL)} FAIL: {FAIL}")
        raise SystemExit(1)
    print("✅ E1-④ 이벤트 캡처 E2E 전부 PASS — hook→CLI→usage.event→events 적재·파생분류·exit_code·무차단 검증")


if __name__ == "__main__":
    main()
