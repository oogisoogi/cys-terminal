#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
javis_phoenix_win_smoke.py — 불사조(무손실 복원) Windows 전용 경량 스모크(S6)

목적: Windows 패리티(javis_phoenix.py S1~S5)를 windows-latest CI 러너에서 결정론·자기완결로 실측한다.
      unix 하네스(javis_phoenix_harness.py)는 macOS 검증 도구로 존치 — 이 스모크는 그와 별개(하네스 무접촉).

케이스(전부 결정론·자기완결·정리 포함):
  ① 경로/파이프 매핑 단위검증 (_win_pipe_slug·_win_state_dir_for_socket — Rust state.rs 규칙 대조)
  ② supervisor(schtasks) 실조작 — throwaway 태스크 create/query/delete + _schtasks_status 분류(실 schtasks·cysd 무접촉·정리)
  ③ 재시작 프리미티브 E2E — 실 cysd(테스트 전용 파이프)→identify→taskkill→파이프 해제 폴링→재기동 유발→pong+boot-epoch delta
  ④ snapshot + 독립 runbook(.ps1) — 실 세대 생성 + MANUAL_RESTORE.ps1 존재·자기완결
  ⑤ stub restore E2E — 기존 --stub surrogate 백엔드로 M9 VERIFIED·COMPLETE(실 데몬·테스트 파이프)
  ⑥ deploy --plan + exit code 계약 — 무실행·exit0·부작용 0

mac(비-Windows)에서 실행 시: "Windows 전용" 정직 안내 후 exit 0(skip). 실 Windows 실측은 CI 러너.

환경:
  PHOENIX_CYS   — cys.exe 절대경로(미설정 시 PATH). PHOENIX_CYSD — cysd.exe 절대경로(미설정 시 cys.exe 형제 → PATH).
  자기 stdout·phoenix 하위프로세스는 utf-8 로 고정(한글 stdout cp1252 크래시 방지·RC-16 관례).
"""

import json
import os
import re
import shutil
import subprocess
import sys
import time

IS_WIN = os.name == "nt"
HERE = os.path.dirname(os.path.abspath(__file__))
PHOENIX = os.path.join(HERE, "javis_phoenix.py")

# 한글 stdout utf-8 고정(standalone 실행 안전 — CI env 비의존)
try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

_FAILS = []
_RESULTS = {}


def log(msg):
    sys.stdout.write("[win-smoke] %s\n" % msg)
    sys.stdout.flush()


def check(name, cond, detail=""):
    ok = bool(cond)
    _RESULTS[name] = ok
    if not ok:
        _FAILS.append(name)
    log(("PASS " if ok else "FAIL ") + name + (" :: " + str(detail)[:200] if detail else ""))
    return ok


def _phoenix_env(extra=None):
    env = dict(os.environ)
    env["PYTHONUTF8"] = "1"            # 한글 stdout/파일 cp1252 크래시 방지(RC-16 관례)
    env["PYTHONIOENCODING"] = "utf-8"
    if extra:
        env.update(extra)
    return env


def resolve_bins():
    cys = os.environ.get("PHOENIX_CYS") or shutil.which("cys") or shutil.which("cys.exe")
    cysd = os.environ.get("PHOENIX_CYSD")
    if not cysd and cys:
        cand = os.path.join(os.path.dirname(cys), "cysd.exe")
        cysd = cand if os.path.exists(cand) else (shutil.which("cysd") or shutil.which("cysd.exe"))
    return cys, cysd


def cys_cli(cys, pipe, *args, timeout=20):
    cmd = [cys]
    if pipe:
        cmd += ["--socket", pipe]
    cmd += [str(a) for a in args]
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=_phoenix_env())
    except subprocess.TimeoutExpired:
        class _R:
            returncode = 124
            stdout = ""
            stderr = "TIMEOUT"
        return _R()


def _ping_pong(cys, pipe):
    r = cys_cli(cys, pipe, "ping", timeout=6)
    return getattr(r, "returncode", 1) == 0 and "pong" in (getattr(r, "stdout", "") or "")


def spawn_test_daemon(cysd, pipe, wait=15.0):
    """테스트 전용 파이프에 cysd 를 기동(CYS_SOCKET 오버라이드 — Rust lib.rs socket_path 존중). ping OK 까지 대기."""
    env = _phoenix_env({"CYS_SOCKET": pipe})
    for k in ("AITERM_SOCKET", "AITERM_SURFACE_ID"):
        env.pop(k, None)
    CREATE_NO_WINDOW = 0x08000000
    p = subprocess.Popen([cysd], env=env, stdin=subprocess.DEVNULL,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                         creationflags=CREATE_NO_WINDOW)
    return p


def _win_state_dir(phoenix, pipe):
    try:
        return phoenix._win_state_dir_for_socket(pipe)
    except Exception:
        return None


def teardown_daemon(cys, pipe, state_dir=None, tracked=None):
    """테스트 데몬 정리 — identify→daemon_pid→taskkill /T /F(재기동된 detached 인스턴스 포함) + 추적 Popen + 상태dir 제거."""
    try:
        r = cys_cli(cys, pipe, "identify", timeout=6)
        txt = getattr(r, "stdout", "") or ""
        i = txt.find("{")
        if i >= 0:
            pid = json.loads(txt[i:]).get("daemon_pid")
            if isinstance(pid, int):
                subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"],
                               capture_output=True, text=True, timeout=10)
    except Exception:
        pass
    if tracked is not None:
        try:
            tracked.kill()
        except Exception:
            pass
    if state_dir:
        shutil.rmtree(state_dir, ignore_errors=True)


# ------------------------------------------------------------------ 케이스

def case1_path_mapping(phoenix):
    log("① 경로/파이프 매핑 단위검증")
    cases = {r"\\.\pipe\cys": "cys", r"\\.\pipe\cys-dept-alpha": "cys-dept-alpha",
             "//./pipe/cys": "cys", r"\\.\pipe\cys_x.1": "cys_x1"}
    for pipe, exp in cases.items():
        check("① slug %s" % exp, phoenix._win_pipe_slug(pipe) == exp, phoenix._win_pipe_slug(pipe))
    la = os.environ.get("LOCALAPPDATA") or ""
    d0 = phoenix._win_state_dir_for_socket(r"\\.\pipe\cys").replace("\\", "/")
    dd = phoenix._win_state_dir_for_socket(r"\\.\pipe\cys-dept-alpha").replace("\\", "/")
    check("① default pipe->%LOCALAPPDATA%\\cys", d0.endswith("/cys") and la.replace("\\", "/").lower() in d0.lower(), d0)
    check("① dept pipe->...\\cys\\cys-dept-alpha", dd.endswith("/cys/cys-dept-alpha"), dd)
    check("① state_dir_for(pipe) 디스패치", phoenix.state_dir_for(r"\\.\pipe\cys-dept-beta").replace("\\", "/").endswith("/cys/cys-dept-beta"))


def case2_schtasks(phoenix):
    log("② supervisor(schtasks) 실조작 — throwaway 태스크(cysd 무접촉·정리)")
    task = "phoenix-win-smoke-tmp"
    # 정리 선행(이전 잔재)
    phoenix._schtasks("/Delete", "/TN", task, "/F")
    cr = phoenix._schtasks("/Create", "/TN", task, "/TR", "cmd /c exit", "/SC", "ONLOGON", "/RL", "LIMITED", "/F")
    check("② schtasks /Create rc0", getattr(cr, "returncode", 1) == 0, getattr(cr, "stderr", ""))
    st_managed = phoenix._schtasks_status(task)
    check("② 등록됨->managed", st_managed.get("state") == "managed", st_managed)
    dl = phoenix._schtasks("/Delete", "/TN", task, "/F")
    check("② schtasks /Delete rc0", getattr(dl, "returncode", 1) == 0, getattr(dl, "stderr", ""))
    st_unmanaged = phoenix._schtasks_status(task)
    check("② 삭제후->unmanaged", st_unmanaged.get("state") == "unmanaged", st_unmanaged)
    # 실 cysd 태스크 상태 조회(읽기 전용·무변경) — 감독자 status 가 크래시 없이 분류하는지
    real = phoenix.supervisor_status("cysd")
    check("② supervisor_status('cysd') 분류(읽기전용)", real.get("supervisor") == "schtasks" and real.get("state") in ("managed", "orphan", "unmanaged"), real)


def case3_restart_primitive(phoenix, cys, cysd):
    log("③ 재시작 프리미티브 E2E (테스트 파이프·identify→taskkill→파이프해제→retrigger→pong+epoch delta)")
    pipe = r"\\.\pipe\cys-phxsmoke-restart"
    sd = _win_state_dir(phoenix, pipe)
    tracked = None
    try:
        shutil.rmtree(sd, ignore_errors=True)
        tracked = spawn_test_daemon(cysd, pipe)
        up = False
        for _ in range(40):
            if _ping_pong(cys, pipe):
                up = True
                break
            time.sleep(0.3)
        if not check("③ 테스트 데몬 기동(ping pong)", up):
            return
        phoenix.CYS = cys
        phoenix.SUPERVISOR_LABEL = "phoenix-smoke-none"   # 존재하지 않는 태스크→unmanaged→cys list lazy-spawn 경로(글로벌 cysd 무접촉)
        epoch1 = phoenix.get_boot_epoch(pipe)
        res = phoenix._win_restart_daemon(pipe, timeout=30)
        check("③ identify→daemon_pid 획득", isinstance(res.get("daemon_pid"), int), res.get("daemon_pid"))
        check("③ taskkill 수행(rc0)", res.get("taskkill_rc") == 0, res.get("taskkill_out"))
        check("③ 파이프 해제(socket_death) 관측 후 retrigger", res.get("socket_death_observed") is True, res.get("retrigger"))
        # 부활 폴링(pong) + boot-epoch delta 확증(조용한 오복원 방어 — 새 세대여야 진짜 재시작)
        revived = False
        for _ in range(80):
            if _ping_pong(cys, pipe):
                revived = True
                break
            time.sleep(0.4)
        epoch2 = phoenix.get_boot_epoch(pipe)
        check("③ 재기동 후 pong 복귀", revived)
        check("③ boot-epoch delta(실제 새 세대 — 조용한 오복원 아님)",
              epoch1 is not None and epoch2 is not None and epoch1 != epoch2, "%s->%s" % (epoch1, epoch2))
    finally:
        teardown_daemon(cys, pipe, state_dir=sd, tracked=tracked)


def case4_snapshot_runbook(phoenix, cys):
    log("④ snapshot + 독립 runbook(.ps1) 자기완결")
    pipe = r"\\.\pipe\cys-phxsmoke-snap"
    sd = _win_state_dir(phoenix, pipe)
    try:
        os.makedirs(sd, exist_ok=True)
        with open(os.path.join(sd, "topology.json"), "w", encoding="utf-8") as f:
            json.dump({"entries": [{"role": "worker", "agent": "claude", "session_id": "S1"},
                                   {"role": "cso", "agent": "claude", "session_id": "S2"}], "updated_at": 0}, f)
        phoenix.CYS = cys
        roster = {"worker": {"agent": "claude"}, "cso": {"agent": "claude"}}
        snap = phoenix._deploy_snapshot(pipe, roster)
        check("④ 세대 스냅샷 생성", snap.get("ok") and snap.get("gen"), snap.get("error") or snap.get("gen"))
        rb = snap.get("runbook") or ""
        check("④ runbook=MANUAL_RESTORE.ps1 존재", rb.endswith("MANUAL_RESTORE.ps1") and os.path.exists(rb), rb)
        body = open(rb, encoding="utf-8-sig").read() if os.path.exists(rb) else ""
        selfcontained = ("cys daemon install" in body and "schtasks /Query /TN" in body
                         and "cys list" in body and "cys ping" in body
                         and all(("--role %s" % r) in body for r in roster)
                         and "javis_phoenix" not in body and "launchctl" not in body)
        check("④ runbook 자기완결(schtasks+cys만·집행/launchctl 미호출)", selfcontained)
        # ★권고#1: LIVE default_sources 가 Windows 에서 %LOCALAPPDATA%\cys 의 데몬 L1 선언상태를 실제로 포착하는지
        #   (unix ~/.local/state 로 조용히 새지 않음). 라이브 데몬 상태를 읽기만 하고 스냅샷은 만들지 않는다(소스 목록 단언).
        import javis_state_snapshot as snap
        la = (os.environ.get("LOCALAPPDATA") or "").replace("\\", "/").lower()
        live_srcs = [s.replace("\\", "/").lower() for s in snap.default_sources()]
        main_ok = bool(la) and any(s == la + "/cys/topology.json" for s in live_srcs)
        no_unix_leak = not any("/.local/state/cys/topology.json" in s for s in live_srcs)
        check("④ 권고#1: LIVE default_sources 가 %LOCALAPPDATA%\\cys L1 상태 포착",
              main_ok and no_unix_leak, "la=%s main_ok=%s no_unix_leak=%s" % (la, main_ok, no_unix_leak))
    finally:
        shutil.rmtree(sd, ignore_errors=True)


def case5_stub_restore(phoenix, cys, cysd):
    log("⑤ stub restore E2E (surrogate 백엔드·M9 VERIFIED·COMPLETE)")
    pipe = r"\\.\pipe\cys-phxsmoke-restore"
    sd = _win_state_dir(phoenix, pipe)
    tracked = None
    try:
        shutil.rmtree(sd, ignore_errors=True)
        tracked = spawn_test_daemon(cysd, pipe)
        up = False
        for _ in range(40):
            if _ping_pong(cys, pipe):
                up = True
                break
            time.sleep(0.3)
        # 데몬 기동 후 topology 시드(상태dir 은 데몬이 생성)
        os.makedirs(sd, exist_ok=True)
        with open(os.path.join(sd, "topology.json"), "w", encoding="utf-8") as f:
            json.dump({"entries": [{"role": "worker", "agent": "stub", "session_id": "SID-W-1",
                                    "cwd": sd, "title": "w"}], "updated_at": 0}, f)
        if not check("⑤ 테스트 데몬 기동", up):
            return
        r = subprocess.run([sys.executable, PHOENIX, "--socket", pipe, "restore", "--ticket", "WS", "--stub"],
                           capture_output=True, text=True, timeout=90, env=_phoenix_env({"PHOENIX_CYS": cys}))
        txt = r.stdout or ""
        i = txt.find("{")
        j = json.loads(txt[i:]) if i >= 0 else {}
        check("⑤ phoenix_restore=VERIFIED", j.get("phoenix_restore") == "VERIFIED", j.get("phoenix_restore"))
        check("⑤ completeness=COMPLETE", j.get("completeness") == "COMPLETE", j.get("completeness"))
    finally:
        teardown_daemon(cys, pipe, state_dir=sd, tracked=tracked)


def case6_deploy_plan(cys):
    log("⑥ deploy --plan + exit code 계약(무실행·exit0·부작용0)")
    pipe = r"\\.\pipe\cys-phxsmoke-plan"
    r = subprocess.run([sys.executable, PHOENIX, "--socket", pipe, "deploy", "--plan", "--ticket", "PLAN", "--stub"],
                       capture_output=True, text=True, timeout=30, env=_phoenix_env({"PHOENIX_CYS": cys}))
    txt = r.stdout or ""
    i = txt.find("{")
    j = json.loads(txt[i:]) if i >= 0 else {}
    check("⑥ --plan exit 0", r.returncode == 0, r.returncode)
    check("⑥ stages 출력(무실행)", j.get("deploy") == "PLAN" and isinstance(j.get("stages"), list) and j.get("stages"), j.get("stages"))


def run():
    if not IS_WIN:
        log("이 스모크는 Windows 전용입니다 — mac/Unix 에서는 skip(정직). 실측은 windows-latest CI 러너.")
        log("(mac 무회귀·Windows 분기 단위검증은 javis_phoenix_harness.py phoenix-p12-deploy/p9-ci 및 별도 단위검증으로 수행)")
        print(json.dumps({"win_smoke": "SKIP(non-windows)", "platform": os.name, "skipped": True},
                         ensure_ascii=False, indent=2))
        return 0

    # phoenix 를 모듈로 임포트(직접호출 케이스 ①③④) + 서브프로세스(⑤⑥)
    import importlib.util
    spec = importlib.util.spec_from_file_location("javis_phoenix", PHOENIX)
    phoenix = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(phoenix)

    cys, cysd = resolve_bins()
    log("cys=%s cysd=%s" % (cys, cysd))
    if not cys or not cysd:
        check("바이너리 해소(cys·cysd)", False, "cys=%s cysd=%s (PHOENIX_CYS/PHOENIX_CYSD 또는 PATH 필요)" % (cys, cysd))
        print(json.dumps({"win_smoke": "FAIL", "failed": _FAILS, "results": _RESULTS}, ensure_ascii=False, indent=2))
        return 1
    phoenix.CYS = cys

    case1_path_mapping(phoenix)
    case2_schtasks(phoenix)
    case3_restart_primitive(phoenix, cys, cysd)
    case4_snapshot_runbook(phoenix, cys)
    case5_stub_restore(phoenix, cys, cysd)
    case6_deploy_plan(cys)

    ok = not _FAILS
    print(json.dumps({"win_smoke": "PASS" if ok else "FAIL", "failed": _FAILS,
                      "results": _RESULTS, "win_smoke_pass": ok}, ensure_ascii=False, indent=2))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(run())
