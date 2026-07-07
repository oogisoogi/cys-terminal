#!/usr/bin/env python3
"""javis_hud_bridge.py — 메타버스 오피스 수집·정규화 브리지 (DESIGN.md §3)

cys 데몬의 이벤트 스트림(`cys events`)과 스냅샷(`cys fleet/status --json`)을
WorldState(§4)로 정규화해 127.0.0.1 전용 HTTP(+SSE)로 내보낸다.

원칙:
  · 파이썬 stdlib만 사용 (외부 의존 0)
  · 읽기 전용 — 쓰기 API 없음, cys에 대한 호출도 관측 명령만 (R4)
  · 소음 필터·코얼레싱 (§6.3) — watchdog 무시, fx 초당 상한, 알림류 중복 병합
  · 이벤트 유실 방어 — --cursor-file 로 시퀀스 영속, 재시작 시 gap 없이 재개

기동:  cys run --scoped -- python3 bin/javis_hud_bridge.py
접속:  http://127.0.0.1:8765
"""
import json
import os
import re
import secrets
import subprocess
import sys
import threading
import time
from collections import defaultdict, deque
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from queue import Empty, Full, Queue

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WEB_DIR = os.path.join(ROOT, "web")
STATE_DIR = os.environ.get("HUD_STATE_DIR") or os.path.join(ROOT, "state")  # 팩 편입 시 외부 지정
BIND = "127.0.0.1"
PORT = int(os.environ.get("HUD_PORT", "8642"))  # 8765는 이 장비에서 선점 실측 → 회피
FLEET_POLL_SECS = float(os.environ.get("HUD_POLL", "2.0"))
CYS = os.environ.get("HUD_CYS_BIN", "cys")
BACKLOG_FX_SECS = 90.0   # 이보다 오래된 이벤트는 상태만 반영, fx(연출) 억제 — 콜드스타트 폭주 방지
CMD_MAX_LEN = 2000       # 조작 지시 텍스트 상한

# ---------------------------------------------------------------- 판정 (§5)
HOOK_ACTIVE_WINDOW = 30.0      # 최근 30s 내 도구 훅 → active
SELF_REPORT_FRESH = 300.0      # 자기보고 신선 기준
IDLE_WAITING = 300
IDLE_DROWSY = 3600
STATE_MAP = {"working": "active", "waiting": "waiting", "quiescing": "quiescing"}


def judge_presence(node, now, last_hook_ts):
    """(presence, confidence) 판정 — DESIGN.md §5 우선순위 표 그대로.

    node: fleet surface dict. last_hook_ts: 해당 surface의 마지막 hook 시각(없으면 None).
    """
    if node.get("exited") or node.get("agent_alive") is False:
        return "dead", 1.0
    if last_hook_ts is not None and (now - last_hook_ts) <= HOOK_ACTIVE_WINDOW:
        return "active", 0.95
    st = node.get("status") or {}
    state = st.get("state")
    age = st.get("age_secs")
    if state in STATE_MAP and age is not None and age < SELF_REPORT_FRESH:
        return STATE_MAP[state], 0.9
    idle = node.get("idle_secs")
    stale_penalty = 0.15 if (state and age is not None and age >= SELF_REPORT_FRESH) else 0.0
    if idle is None:
        return "unknown", 0.3
    if idle < IDLE_WAITING:
        return "waiting", 0.7 - stale_penalty
    if idle < IDLE_DROWSY:
        return "drowsy", 0.7 - stale_penalty
    return "sleeping", 0.7 - stale_penalty


def compute_activity(hook_count_60s, lines_per_sec):
    """활동 강도 0..1 (§7-3): hook 빈도와 출력량 중 큰 쪽. 상한 정규화."""
    h = min(hook_count_60s / 20.0, 1.0)
    l = min(max(lines_per_sec, 0.0) / 15.0, 1.0)
    return round(max(h, l), 3)


def pick_ctx(node):
    """ctx% 선택: 실측(usage.ctx_pct·statusline) > 자기보고(status.context_pct)."""
    u = node.get("usage") or {}
    if isinstance(u.get("ctx_pct"), (int, float)):
        return u["ctx_pct"]
    st = node.get("status") or {}
    if isinstance(st.get("context_pct"), (int, float)):
        return st["context_pct"]
    return None


# ------------------------------------------------------------ 코얼레싱 (§6.3)
NOISE_NAMES = {"watchdog.duplicate_procs", "watchdog.proc_count"}
ALERT_COALESCE = {  # (이벤트명 → 동일 surface 재발화 억제 윈도 s)
    "health.alert": 30.0,
    "master.deadman": 60.0,
    "pane.idle": 120.0,
    "schedule.fired": 10.0,
    "schedule.error": 30.0,
}
FX_BUDGET_PER_SEC = 20


class Coalescer:
    """알림류 (name,surface) 윈도 병합 + fx 전역 초당 예산. 상태(patch)는 여기 안 거침."""

    def __init__(self, budget=FX_BUDGET_PER_SEC):
        self.last_seen = {}
        self.budget = budget
        self.bucket = budget
        self.bucket_ts = None  # 첫 allow의 now 기준으로 지연 초기화 (시간축 혼선 방지)

    def allow(self, name, surface_id, now=None):
        now = time.monotonic() if now is None else now
        if self.bucket_ts is None:
            self.bucket_ts = now
        # 전역 예산 (초당 리필 · 음수 경과 가드)
        self.bucket = min(self.budget,
                          self.bucket + max(0.0, now - self.bucket_ts) * self.budget)
        self.bucket_ts = now
        if self.bucket < 1:
            return False
        win = ALERT_COALESCE.get(name)
        if win is not None:
            key = (name, surface_id)
            last = self.last_seen.get(key)
            if last is not None and (now - last) < win:
                return False
            self.last_seen[key] = now
        self.bucket -= 1
        return True


# ------------------------------------------------------- 조작 게이트 (T3 · §R4 해제)
# 위협모델: 실경계 = 브라우저 안 원격 웹페이지의 localhost CSRF (로컬 프로세스는
#   이미 cys를 직접 호출 가능하므로 경계 밖 — 이 게이트의 목적이 아니다).
# 방어: 기동 시 랜덤 토큰(HTML 주입) + 커스텀 헤더 X-HUD-Token(=preflight 강제,
#   OPTIONS는 무조건 deny) + Origin allowlist + 대상 surface allowlist(실존만)
#   + 제어문자·개행 제거 + 길이 상한 + fail-closed + 감사 로그.
# 근본한계(명문화 · 원칙5):
#   ① 대상 surface가 shell 프롬프트 상태면 Return 주입 = 즉시 실행이다. 이는 기능
#      그 자체이며 신뢰 주체(오너)의 실수 방지를 위해 개행 제거·1줄 강제만 한다.
#   ② GET / 를 읽을 수 있는 로컬 주체는 토큰을 얻는다 — localhost 바인딩이 경계.
CMD_KEY_RE = re.compile(r"^surface:\d{1,8}$")
CTRL_CHARS = re.compile(r"[\x00-\x1f\x7f]")


def allowed_origins(port):
    return {f"http://127.0.0.1:{port}", f"http://localhost:{port}",
            "tauri://localhost", "http://tauri.localhost"}


def gate_command(body, token_header, expected_token, origin, origins, known_keys):
    """조작 요청 판정 (순수 함수 — 자기공격 테스트 대상).

    반환 (ok, err, cleaned): ok=False면 err=거부 사유. deny-by-default —
    모든 검사를 통과한 요청만 allow, 판정 불능은 전부 거부(fail-closed).
    """
    if not expected_token or not token_header or \
       not secrets.compare_digest(str(token_header), str(expected_token)):
        return False, "bad_token", None
    if origin is not None and origin not in origins:
        return False, "bad_origin", None            # 교차출처 브라우저 확정 신호
    if not isinstance(body, dict):
        return False, "bad_body", None
    key = body.get("key")
    if not isinstance(key, str) or not CMD_KEY_RE.match(key):
        return False, "bad_key_format", None
    if key not in known_keys:
        return False, "unknown_target", None        # 실존 fleet allowlist 밖 → deny
    text = body.get("text")
    if not isinstance(text, str):
        return False, "bad_text", None
    text = CTRL_CHARS.sub(" ", text).strip()        # 제어문자·개행 → 공백 (1줄 강제)
    if not text or len(text) > CMD_MAX_LEN:
        return False, "bad_text_len", None
    return True, None, {"key": key, "text": text}


# ------------------------------------------------------------------ 월드
TODO_ROLE = re.compile(r"(?:^|/)(?P<r>[A-Z_]+)_TODO\.md$")
TODO_ROLE_MAP = {"MASTER": "master", "WORKER": "worker", "CSO": "cso",
                 "REVIEWER_GEMINI": "reviewer-gemini", "REVIEWER_AGY": "reviewer-agy",
                 "REVIEWER_CODEX": "reviewer-codex"}


class World:
    """WorldState 보유·병합·직렬화 (계약 v1 · §4)."""

    def __init__(self):
        self.lock = threading.Lock()
        self.departments = []          # fleet 원본 (surfaces 포함)
        self.daemon = {}
        self.todo = {}                 # path → {done,total,age_secs}
        self.seq = 0
        self.hooks = defaultdict(deque)      # surface_id → 최근 hook ts deque
        self.line_hist = {}            # surface_id → (line_count, ts)
        self.line_rate = {}            # surface_id → lines/s
        self.flags = defaultdict(dict)  # surface_id → {flag: expiry_ts}
        self.server_room = {}          # pid → {cmd, scoped}
        self.prev_nodes = {}           # key → 직전 노드 스냅샷 (diff용)

    # --- 수집 반영 ---
    def note_hook(self, surface_id, ts):
        dq = self.hooks[surface_id]
        dq.append(ts)
        while dq and ts - dq[0] > 60.0:
            dq.popleft()

    def set_flag(self, surface_id, flag, ttl):
        self.flags[surface_id][flag] = time.time() + ttl

    def live_flags(self, surface_id, now):
        f = self.flags.get(surface_id) or {}
        return sorted(k for k, exp in f.items() if exp > now)

    def apply_usage(self, surface_id, payload):
        """usage.updated → 해당 노드 usage 즉시 갱신. 성공 시 노드 key 반환."""
        with self.lock:
            for d in self.departments:
                for s in d.get("surfaces", []):
                    if s.get("surface_id") == surface_id:
                        s["usage"] = payload
                        return s.get("surface_ref")
        return None

    def apply_todo(self, path, done, total):
        with self.lock:
            ent = self.todo.setdefault(path, {})
            ent["done"], ent["total"], ent["age_secs"] = done, total, 0

    def apply_ledger(self, name, payload):
        if name == "ledger.registered":
            self.server_room[payload.get("pid")] = {
                "cmd": (payload.get("cmd") or "")[:80], "scoped": payload.get("scoped")}
        elif name == "ledger.killed":
            self.server_room.pop(payload.get("pid"), None)

    def merge_fleet(self, fleet, status):
        """스냅샷 병합 → (patch 프레임 목록, 구조변화 여부). 부재 시 이전 유지."""
        with self.lock:
            if status:
                self.daemon = status.get("daemon") or self.daemon
                self.daemon["paused"] = status.get("paused", False)
                for p, t in (status.get("todo") or {}).items():
                    self.todo[p] = t
                self.seq = self.daemon.get("latest_seq", self.seq)
            structural = False
            if fleet:
                new_deps = fleet.get("departments") or []
                old_keys = {s.get("surface_ref") for d in self.departments
                            for s in d.get("surfaces", [])}
                new_keys = {s.get("surface_ref") for d in new_deps
                            for s in d.get("surfaces", [])}
                old_shape = [d.get("department") for d in self.departments]
                new_shape = [d.get("department") for d in new_deps]
                structural = (old_keys != new_keys) or (old_shape != new_shape)
                # 출력량 변화율 (line_count 델타)
                now = time.time()
                for d in new_deps:
                    for s in d.get("surfaces", []):
                        sid = s.get("surface_id")
                        lc = s.get("line_count") or 0
                        prev = self.line_hist.get(sid)
                        if prev and now > prev[1]:
                            self.line_rate[sid] = max(0.0, (lc - prev[0]) / (now - prev[1]))
                        self.line_hist[sid] = (lc, now)
                self.departments = new_deps
            patches = self._diff_patches()
            return patches, structural

    def _node_view(self, s, now):
        """fleet surface → 계약 v1 노드 뷰."""
        sid = s.get("surface_id")
        dq = self.hooks.get(sid) or ()
        last_hook = dq[-1] if dq else None
        presence, conf = judge_presence(s, now, last_hook)
        st = s.get("status") or {}
        ctx = pick_ctx(s)
        u = s.get("usage") or {}
        flags = self.live_flags(sid, now)
        if ctx is not None and ctx >= 90:
            flags = flags + ["ctx_critical"]
        return {
            "key": s.get("surface_ref"), "surface_id": sid,
            "role": s.get("role") or "무소속",
            "agent": s.get("agent") or u.get("agent"),
            "presence": presence, "presence_conf": round(conf, 2),
            "task": (st.get("task") or "")[:80],
            # 60s 버킷 양자화 — 매초 변하는 idle이 patch를 스팸하지 않도록 (§6.3)
            "idle_secs": (s.get("idle_secs") // 60) * 60
                         if isinstance(s.get("idle_secs"), int) else s.get("idle_secs"),
            "ctx": {"pct": ctx, "window": u.get("ctx_window")},
            "rate": [{"label": r.get("label"), "used_pct": r.get("used_pct"),
                      "resets_at": r.get("resets_at")}
                     for r in (u.get("rate") or [])],
            "activity": compute_activity(len(dq), self.line_rate.get(sid, 0.0)),
            "flags": flags,
        }

    def _diff_patches(self):
        """노드 뷰 diff → patch 프레임 목록. (lock 보유 상태에서 호출)"""
        now = time.time()
        patches = []
        seen = {}
        for d in self.departments:
            for s in d.get("surfaces", []):
                v = self._node_view(s, now)
                seen[v["key"]] = v
                old = self.prev_nodes.get(v["key"])
                if old != v:
                    patches.append({"t": "patch", "key": v["key"], "node": v})
        self.prev_nodes = seen
        return patches

    def known_keys(self):
        """현재 fleet에 실존하는 surface_ref 집합 (조작 대상 allowlist)."""
        with self.lock:
            return {s.get("surface_ref") for d in self.departments
                    for s in d.get("surfaces", []) if s.get("surface_ref")}

    def snapshot(self):
        """전체 WorldState (계약 v1)."""
        with self.lock:
            now = time.time()
            deps = []
            unassigned = []
            for i, d in enumerate(self.departments):
                nodes = []
                for s in d.get("surfaces", []):
                    v = self._node_view(s, now)
                    if v["role"] == "무소속":
                        unassigned.append(v)
                    else:
                        nodes.append(v)
                deps.append({
                    "id": d.get("department"),
                    "floor": len(self.departments) - i,   # 첫 부서(본부)가 최상층
                    "nodes": nodes,
                })
            todo_named = {}
            for p, t in self.todo.items():
                m = TODO_ROLE.search(p or "")
                if m:
                    todo_named[TODO_ROLE_MAP.get(m.group("r"), m.group("r"))] = t
            return {
                "v": 1, "ts": now, "seq": self.seq,
                "daemon": {"version": self.daemon.get("version"),
                           "paused": bool(self.daemon.get("paused"))},
                "departments": deps,
                "todo": todo_named,
                "lobby": {"unassigned": unassigned},
                "server_room": [dict(pid=k, **v) for k, v in sorted(self.server_room.items())],
            }


# ------------------------------------------------------------- 브로드캐스트
class Hub:
    """SSE 클라이언트 큐 관리. 큐 포화 시 world 재동기화로 강등(상태 유실 방지)."""

    def __init__(self):
        self.lock = threading.Lock()
        self.clients = set()

    def attach(self):
        q = Queue(maxsize=500)
        with self.lock:
            self.clients.add(q)
        return q

    def detach(self, q):
        with self.lock:
            self.clients.discard(q)

    def publish(self, frame):
        data = json.dumps(frame, ensure_ascii=False)
        with self.lock:
            clients = list(self.clients)
        for q in clients:
            try:
                q.put_nowait(data)
            except Full:
                try:  # 밀린 클라이언트: 비우고 재동기화 표식
                    while True:
                        q.get_nowait()
                except Empty:
                    pass
                try:
                    q.put_nowait(json.dumps({"t": "resync"}))
                except Full:
                    pass


# ---------------------------------------------------------- 이벤트 → 프레임
TOOL_HOOKS = {"agent.hook.PreToolUse": "pre", "agent.hook.PostToolUse": "post",
              "agent.hook.PermissionRequest": "perm"}


def route_event(ev, world, coal, now=None):
    """데몬 이벤트 1건 → 방출 프레임 목록 (+월드 반영). 순수 로직 (테스트 대상).

    반환: (frames, poke_fleet) — poke_fleet=True면 즉시 스냅샷 재폴링 필요.
    백로그 게이트(§7): 이벤트가 BACKLOG_FX_SECS보다 과거면 상태 반영만 하고
    fx 프레임은 억제한다 — "켜는 순간"의 과거 연출 폭주 방지, 상태는 손실 0.
    """
    name = ev.get("name") or ""
    if name in NOISE_NAMES or name.startswith("watchdog."):
        return [], False
    now = time.time() if now is None else now
    ts = ev.get("timestamp") or now
    backlog = (now - ts) > BACKLOG_FX_SECS
    sid = ev.get("surface_id")
    p = ev.get("payload") or {}
    key = f"surface:{sid}" if sid is not None else None
    frames = []
    poke = False

    if name in TOOL_HOOKS:
        now = ev.get("timestamp") or time.time()
        if sid is not None:
            world.note_hook(sid, now)
        phase = TOOL_HOOKS[name]
        if phase == "perm" and not p.get("is_actionable"):
            phase = "pre"  # 비실행성 권한훅은 도구 연출로 강등
        if coal.allow(name, sid):
            frames.append({"t": "fx", "kind": "tool", "key": key, "phase": phase,
                           "tool": p.get("tool_name"), "role": p.get("role"),
                           "ok": (p.get("exit_code") in (0, None))})
    elif name == "usage.updated":
        k = world.apply_usage(sid, p)
        if k:
            frames.append({"t": "fx", "kind": "usage", "key": k, "pct": p.get("ctx_pct")})
    elif name == "todo.updated":
        world.apply_todo(p.get("path"), p.get("done"), p.get("total"))
        frames.append({"t": "fx", "kind": "todo", "path": p.get("path"),
                       "done": p.get("done"), "total": p.get("total")})
    elif name == "surface.input_injected":
        frames.append({"t": "fx", "kind": "doc", "to": key,
                       "from": f"surface:{p.get('from')}" if p.get("from") is not None else None,
                       "bytes": p.get("bytes")})
    elif name == "queue.enqueued":
        frames.append({"t": "fx", "kind": "queue", "to": key, "depth": p.get("depth")})
    elif name == "queue.delivered":
        frames.append({"t": "fx", "kind": "doc", "to": key, "from": None,
                       "bytes": p.get("bytes")})
    elif name in ("surface.created", "surface.closed", "surface.exited"):
        poke = True
        frames.append({"t": "fx", "kind": "spawn" if name == "surface.created" else "despawn",
                       "key": key or f"surface:{(p.get('surface_ref') or '').split(':')[-1]}"})
    elif name == "health.alert":
        rule = p.get("rule") or "alert"
        if sid is not None and rule == "rate_limited":
            world.set_flag(sid, "rate_limited", 90)
        if coal.allow(name, sid):
            frames.append({"t": "fx", "kind": "alert", "key": key, "rule": rule,
                           "line": (p.get("line") or "")[:120]})
    elif name == "master.deadman":
        if coal.allow(name, sid):
            frames.append({"t": "fx", "kind": "deadman", "key": key,
                           "idle_secs": p.get("idle_secs")})
    elif name in ("schedule.fired", "schedule.error"):
        if coal.allow(name, p.get("job_id")):
            frames.append({"t": "fx", "kind": "bell", "job": p.get("job_id"),
                           "error": name.endswith("error")})
    elif name in ("ledger.registered", "ledger.killed"):
        world.apply_ledger(name, p)
        frames.append({"t": "fx", "kind": "rack", "on": name == "ledger.registered",
                       "pid": p.get("pid")})
    elif name == "pane.idle":
        if coal.allow(name, sid):
            frames.append({"t": "fx", "kind": "idle", "key": key,
                           "idle_secs": p.get("idle_seconds")})
    if backlog:  # 과거 이벤트: 상태 반영은 위에서 완료, 연출만 억제
        frames = [fr for fr in frames if fr.get("t") != "fx"]
    return frames, poke


# ------------------------------------------------------------------ 수집 루프
ARCHIVE_PATH = os.path.join(STATE_DIR, "fx_archive.jsonl")
ARCHIVE_CAP = 20 * 1024 * 1024   # 20MB 넘으면 뒤 절반 유지


def archive_fx(ts, frame):
    """타임머신(#4) 토대 — 연출 프레임을 시각과 함께 영속 (연출 리플레이 전용)."""
    try:
        os.makedirs(STATE_DIR, exist_ok=True)
        with open(ARCHIVE_PATH, "a") as f:
            f.write(json.dumps({"ts": ts or time.time(), "fx": frame},
                               ensure_ascii=False) + "\n")
        if os.path.getsize(ARCHIVE_PATH) > ARCHIVE_CAP:
            with open(ARCHIVE_PATH) as f:
                lines = f.readlines()
            with open(ARCHIVE_PATH, "w") as f:
                f.writelines(lines[len(lines)//2:])
    except OSError:
        pass


def read_history(after_ts, limit=6000):
    out = []
    try:
        with open(ARCHIVE_PATH) as f:
            for line in f:
                try:
                    e = json.loads(line)
                except ValueError:
                    continue
                if e.get("ts", 0) > after_ts:
                    out.append(e)
    except OSError:
        pass
    return out[-limit:]


ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07")


def peek_surface(key, lines=12):
    """직원 응답 회수(#2 대화) — read-screen 단발 조회 (상시 폴링 아님·관측 전용)."""
    try:
        r = subprocess.run([CYS, "read-screen", "--surface", key, "--lines", "40"],
                           capture_output=True, text=True, timeout=10)
        if r.returncode != 0:
            return None
        txt = ANSI_RE.sub("", r.stdout)
        rows = [ln.rstrip() for ln in txt.splitlines() if ln.strip()]
        return rows[-lines:]
    except Exception:
        return None


def run_json(args, timeout=10):
    try:
        out = subprocess.run([CYS] + args, capture_output=True, text=True, timeout=timeout)
        if out.returncode != 0:
            return None
        return json.loads(out.stdout)
    except Exception:
        return None


def fleet_loop(world, hub, poke):
    while True:
        fleet = run_json(["fleet", "--json"])
        status = run_json(["status", "--json"])
        patches, structural = world.merge_fleet(fleet, status)
        if structural:
            hub.publish({"t": "world", "world": world.snapshot()})
        else:
            for fr in patches:
                hub.publish(fr)
        poke.wait(FLEET_POLL_SECS)
        poke.clear()


def events_loop(world, hub, poke):
    os.makedirs(STATE_DIR, exist_ok=True)
    cursor = os.path.join(STATE_DIR, "cursor.seq")
    coal = Coalescer()
    while True:
        proc = subprocess.Popen(
            [CYS, "events", "--reconnect", "--cursor-file", cursor],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
        try:
            for line in proc.stdout:
                try:
                    ev = json.loads(line)
                except ValueError:
                    continue
                if ev.get("type") != "event":
                    continue
                world.seq = max(world.seq, ev.get("seq") or 0)
                frames, want_poke = route_event(ev, world, coal)
                for fr in frames:
                    hub.publish(fr)
                    if fr.get("t") == "fx":
                        archive_fx(ev.get("timestamp"), fr)
                if want_poke:
                    poke.set()
        finally:
            proc.terminate()
        time.sleep(2.0)  # 구독 재수립 백오프


# ------------------------------------------------------------------ HTTP
class Handler(BaseHTTPRequestHandler):
    world = None
    hub = None
    routes = {}
    token = None
    origins = set()

    def log_message(self, *a):  # 조용히
        pass

    def _send(self, code, ctype, body, cache=False):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        if cache:
            self.send_header("Cache-Control", "max-age=86400")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = self.path.split("?")[0]
        if path == "/world":
            body = json.dumps(self.world.snapshot(), ensure_ascii=False).encode()
            return self._send(200, "application/json; charset=utf-8", body)
        if path == "/stream":
            return self._sse()
        if path == "/history":
            try:
                after = float((self.path.split("after=")[1].split("&")[0])
                              if "after=" in self.path else 0)
            except ValueError:
                after = 0.0
            body = json.dumps({"events": read_history(after)}, ensure_ascii=False).encode()
            return self._send(200, "application/json; charset=utf-8", body)
        if path == "/peek":
            tok = self.headers.get("X-HUD-Token")
            if not self.token or not tok or not secrets.compare_digest(str(tok), str(self.token)):
                return self._send(403, "application/json", b'{"ok": false, "error": "bad_token"}')
            key = (self.path.split("key=")[1].split("&")[0]) if "key=" in self.path else ""
            if not CMD_KEY_RE.match(key) or key not in self.world.known_keys():
                return self._send(403, "application/json", b'{"ok": false, "error": "bad_key"}')
            rows = peek_surface(key)
            body = json.dumps({"ok": rows is not None, "lines": rows or []},
                              ensure_ascii=False).encode()
            return self._send(200, "application/json; charset=utf-8", body)
        if path in self.routes:
            fp, ctype, cache = self.routes[path]
            try:
                with open(fp, "rb") as f:
                    body = f.read()
                if ctype.startswith("text/html"):   # 조작 토큰 주입 (동일 페이지 한정)
                    body = body.replace(b"__HUD_TOKEN__", (self.token or "").encode())
                return self._send(200, ctype, body, cache)
            except OSError:
                return self._send(404, "text/plain", b"missing asset")
        if path == "/favicon.ico":
            return self._send(204, "text/plain", b"")
        return self._send(404, "text/plain", b"not found")

    def do_OPTIONS(self):  # preflight 무조건 deny → 교차출처 커스텀헤더 요청 차단
        self._send(403, "text/plain", b"denied")

    def do_POST(self):
        if self.path.split("?")[0] != "/command":
            return self._send(405, "text/plain", b"unsupported")
        try:
            n = int(self.headers.get("Content-Length") or 0)
            if n <= 0 or n > 65536:
                raise ValueError
            body = json.loads(self.rfile.read(n))
        except Exception:
            return self._cmd_result(400, False, "bad_json", None)   # fail-closed
        ok, err, cleaned = gate_command(
            body, self.headers.get("X-HUD-Token"), self.token,
            self.headers.get("Origin"), self.origins, self.world.known_keys())
        if not ok:
            return self._cmd_result(403, False, err, body if isinstance(body, dict) else None)
        key, text = cleaned["key"], cleaned["text"]
        # 동일화 경로: cys send(타이핑) → send-key Return(확정). argv 배열 직접
        # 전달이라 shell 해석 표면 없음. 주입은 surface.input_injected 이벤트로
        # 데몬→브리지→SSE로 되돌아와 화면에 배달 연출로 확인된다(동시성).
        r1 = subprocess.run([CYS, "send", "--surface", key, "--", text],
                            capture_output=True, text=True, timeout=10)
        if r1.returncode != 0:
            return self._cmd_result(502, False,
                                    "send_failed: " + (r1.stderr or "")[:120], cleaned)
        r2 = subprocess.run([CYS, "send-key", "--surface", key, "Return"],
                            capture_output=True, text=True, timeout=10)
        if r2.returncode != 0:
            return self._cmd_result(502, False,
                                    "return_failed: " + (r2.stderr or "")[:120], cleaned)
        return self._cmd_result(200, True, None, cleaned)

    def _cmd_result(self, code, ok, err, req):
        try:  # 감사 로그 (원칙: 모든 조작 시도 기록 — allow·deny 불문)
            os.makedirs(STATE_DIR, exist_ok=True)
            with open(os.path.join(STATE_DIR, "command_audit.jsonl"), "a") as f:
                f.write(json.dumps({
                    "ts": time.time(), "ok": ok, "err": err,
                    "origin": self.headers.get("Origin"),
                    "key": (req or {}).get("key"),
                    "text": (req or {}).get("text"),
                }, ensure_ascii=False) + "\n")
        except OSError:
            pass
        body = json.dumps({"ok": ok, "error": err}).encode()
        self._send(code, "application/json; charset=utf-8", body)

    do_PUT = do_DELETE = do_PATCH = lambda self: self._send(405, "text/plain", b"unsupported")

    def _sse(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream; charset=utf-8")
        self.send_header("Cache-Control", "no-cache")
        self.end_headers()
        q = self.hub.attach()
        try:
            first = json.dumps({"t": "world", "world": self.world.snapshot()},
                               ensure_ascii=False)
            self.wfile.write(f"data: {first}\n\n".encode())
            self.wfile.flush()
            while True:
                try:
                    data = q.get(timeout=15.0)
                    self.wfile.write(f"data: {data}\n\n".encode())
                except Empty:
                    self.wfile.write(b": keepalive\n\n")
                self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError, OSError):
            pass
        finally:
            self.hub.detach(q)


def main():
    world = World()
    hub = Hub()
    poke = threading.Event()
    os.makedirs(STATE_DIR, exist_ok=True)
    token = secrets.token_hex(16)          # 세션 토큰 — 기동마다 회전
    tk_path = os.path.join(STATE_DIR, "token")
    with open(tk_path, "w") as f:
        f.write(token)
    os.chmod(tk_path, 0o600)               # state 내부 신규 파일 권한 (게이트 자기보호)
    Handler.world = world
    Handler.hub = hub
    Handler.token = token
    Handler.origins = allowed_origins(PORT)
    Handler.routes = {
        "/": (os.path.join(WEB_DIR, "office3d.html"), "text/html; charset=utf-8", False),
        "/vendor/three.module.js":
            (os.path.join(WEB_DIR, "vendor", "three.module.js"),
             "text/javascript; charset=utf-8", True),
    }
    threading.Thread(target=fleet_loop, args=(world, hub, poke), daemon=True).start()
    threading.Thread(target=events_loop, args=(world, hub, poke), daemon=True).start()
    srv = ThreadingHTTPServer((BIND, PORT), Handler)
    print(f"[hud-bridge] http://{BIND}:{PORT}  (읽기 전용 · 127.0.0.1 한정)", flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
