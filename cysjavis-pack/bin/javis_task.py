#!/usr/bin/env python3
"""javis_task.py — P0-1 원자적 태스크 체크아웃 (Paperclip 계약의 클린룸 파일 포트)

계약(출처: _research/Paperclip_박사급_연구보고서.md §4 P0-1):
- 체크아웃은 원자적이다: 락 획득은 POSIX mkdir(원자적)로, 경쟁 시 정확히 1명만 승리.
- 충돌 = 409(exit 9): 살아있는 소유자가 있다는 뜻 — 재시도 금지(Never retry a 409).
- stale 판정은 시간 TTL이 아니라 run-liveness: 소유 pid가 죽었을 때만 락을 회수(adopt).
  adopt도 원자적: 기존 락 디렉터리 rename(원자적, 1명만 성공) 후 새로 획득.
- blocker는 "done"만 해소로 친다(cancelled는 미해소 잔존). 미해소 blocker가 있으면 체크아웃 거부(exit 4).
- 자동생성 중복 차단: 같은 (origin_kind, origin_fingerprint)의 열린 태스크가 있으면 create 거부(exit 8).
- 태스크가 done이 되면, 그 태스크를 기다리던 태스크들 중 blocker가 전부 해소된 것을 보고한다
  (wakeup 큐 연동은 javis_wakeup.py 몫 — 여기선 unblocked 목록 출력만).

상태 저장: $JAVIS_ROOT/_round/tasks/<id>.json (쓰기는 temp+os.replace 원자적)
락:        $JAVIS_ROOT/_round/tasks/<id>.lock/ (디렉터리 = mkdir 원자성) + owner.json

exit codes: 0 ok · 2 usage · 3 not found · 4 blocked · 8 duplicate origin · 9 conflict(409)
"""
import argparse
import getpass
import json
import os
import socket
import sys
import time
import uuid

ROOT = os.environ.get("JAVIS_ROOT", "/Users/cys/Desktop/CYSjavis")
TASKS_DIR = os.path.join(ROOT, "_round", "tasks")

STATUSES = ["backlog", "todo", "in_progress", "in_review", "done", "blocked", "cancelled"]
OPEN_STATUSES = ["backlog", "todo", "in_progress", "in_review", "blocked"]
TERMINAL_STATUSES = ["done", "cancelled"]

EXIT_OK, EXIT_USAGE, EXIT_NOTFOUND, EXIT_BLOCKED, EXIT_DUP, EXIT_CONFLICT = 0, 2, 3, 4, 8, 9


def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%S%z")


def _task_path(task_id):
    return os.path.join(TASKS_DIR, f"{task_id}.json")


def _lock_dir(task_id):
    return os.path.join(TASKS_DIR, f"{task_id}.lock")


def _write_json_atomic(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = f"{path}.tmp.{os.getpid()}.{uuid.uuid4().hex[:8]}"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)  # 원자적 교체


def _read_task(task_id):
    try:
        with open(_task_path(task_id), encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        return None


def _list_tasks():
    if not os.path.isdir(TASKS_DIR):
        return []
    out = []
    for name in sorted(os.listdir(TASKS_DIR)):
        if name.endswith(".json") and not name.startswith("."):
            t = _read_task(name[:-5])
            if t:
                out.append(t)
    return out


def _pid_alive(pid):
    """run-liveness: pid 생존 확인. 테스트 주입용 JAVIS_TASK_LIVENESS=alive|dead 지원."""
    override = os.environ.get("JAVIS_TASK_LIVENESS")
    if override == "alive":
        return True
    if override == "dead":
        return False
    try:
        os.kill(int(pid), 0)
        return True
    except (ProcessLookupError, ValueError, TypeError):
        return False
    except PermissionError:
        return True  # 존재하지만 남의 프로세스


def _read_owner(task_id):
    try:
        with open(os.path.join(_lock_dir(task_id), "owner.json"), encoding="utf-8") as f:
            return json.load(f)
    except (FileNotFoundError, NotADirectoryError, json.JSONDecodeError):
        return None


OWNER_WRITE_GRACE_SEC = 5.0  # 락 획득~owner.json 기록 사이의 정당한 창


def _write_owner(lock, rec):
    """owner.json을 락 디렉터리 '재생성 없이' 기록. 락이 탈취돼 사라졌으면 False(경쟁 패배)."""
    tmp = os.path.join(TASKS_DIR, f".owner.tmp.{os.getpid()}.{uuid.uuid4().hex[:8]}")
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(rec, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    try:
        os.replace(tmp, os.path.join(lock, "owner.json"))
        return True
    except (FileNotFoundError, NotADirectoryError, OSError):
        try:
            os.remove(tmp)
        except OSError:
            pass
        return False


def _acquire_lock(task_id, owner_id, pid=None):
    """mkdir 원자성 기반 CAS. 반환: ('acquired'|'conflict'|'adopted', owner_dict|None)

    pid 기본값은 호출자(부모 프로세스) — CLI 자신의 pid는 즉시 종료되어 liveness 근거가
    못 되기 때문. 장수 프로세스(워커 pane 등)에 묶으려면 --pid로 명시.
    """
    lock = _lock_dir(task_id)
    owner_rec = {
        "owner_id": owner_id,
        "pid": pid if pid is not None else os.getppid(),
        "host": socket.gethostname(),
        "user": getpass.getuser(),
        "acquired_at": _now(),
    }
    try:
        os.mkdir(lock)  # ← 원자적: 경쟁 시 1명만 성공
        if not _write_owner(lock, owner_rec):
            return "conflict", None  # 기록 창에서 락 탈취됨 — 경쟁 패배로 처리
        return "acquired", owner_rec
    except FileExistsError:
        pass
    holder = _read_owner(task_id)
    if holder and holder.get("owner_id") == owner_id:
        return "acquired", holder  # 자기 재진입(멱등)
    if holder and _pid_alive(holder.get("pid")):
        return "conflict", holder  # 살아있는 소유자 = 409
    if holder is None:
        # owner.json 미기록: 방금 획득한 경쟁자의 "기록 창"일 수 있음 — grace 이내면 409
        try:
            age = time.time() - os.stat(lock).st_mtime
        except OSError:
            age = OWNER_WRITE_GRACE_SEC  # 락이 사라짐 = 경쟁 진행 중
        if age < OWNER_WRITE_GRACE_SEC:
            return "conflict", None
    # stale(소유 pid 사망, 또는 grace 지난 owner.json 훼손) → 원자적 adopt: rename은 1명만 성공
    stale_name = f"{lock}.stale.{time.time_ns()}.{uuid.uuid4().hex[:6]}"
    try:
        os.rename(lock, stale_name)
    except (FileNotFoundError, OSError):
        return "conflict", _read_owner(task_id)  # 경쟁자가 먼저 adopt함 — 409로 취급
    try:
        os.mkdir(lock)
    except FileExistsError:
        return "conflict", _read_owner(task_id)
    if not _write_owner(lock, owner_rec):
        return "conflict", None
    # stale 잔해는 감사용으로 보존하지 않고 정리(내용은 owner.json 하나)
    try:
        p = os.path.join(stale_name, "owner.json")
        if os.path.exists(p):
            os.remove(p)
        os.rmdir(stale_name)
    except OSError:
        pass
    return "adopted", owner_rec


def _release_lock(task_id, owner_id, force=False):
    lock = _lock_dir(task_id)
    holder = _read_owner(task_id)
    if not os.path.isdir(lock):
        return True
    if not force and holder and holder.get("owner_id") != owner_id:
        return False
    try:
        p = os.path.join(lock, "owner.json")
        if os.path.exists(p):
            os.remove(p)
        os.rmdir(lock)
    except OSError:
        return False
    return True


def _unresolved_blockers(task):
    """blocker는 done만 해소(cancelled는 미해소 잔존 — Paperclip 계약 준수)."""
    out = []
    for bid in task.get("blocked_by", []):
        b = _read_task(bid)
        if b is None or b.get("status") != "done":
            out.append(bid)
    return out


def _find_unblocked_dependents(done_task_id):
    """방금 done이 된 태스크로 인해 blocker가 전부 해소된 열린 태스크 목록."""
    out = []
    for t in _list_tasks():
        if t.get("status") in OPEN_STATUSES and done_task_id in t.get("blocked_by", []):
            if not _unresolved_blockers(t):
                out.append(t["id"])
    return out


def cmd_create(a):
    task_id = a.id or f"T{time.strftime('%m%d')}-{uuid.uuid4().hex[:6]}"
    if _read_task(task_id):
        print(f"error: task exists: {task_id}", file=sys.stderr)
        return EXIT_DUP
    if a.origin_fingerprint:
        for t in _list_tasks():
            if (t.get("status") in OPEN_STATUSES
                    and t.get("origin_kind") == a.origin_kind
                    and t.get("origin_fingerprint") == a.origin_fingerprint):
                print(f"duplicate-origin: open task {t['id']} has same "
                      f"({a.origin_kind},{a.origin_fingerprint}) — create 거부", file=sys.stderr)
                return EXIT_DUP
    task = {
        "id": task_id,
        "title": a.title,
        "status": a.status,
        "why": a.why or "",           # goal 체인: 이 일이 왜 존재하는가
        "goal": a.goal or "",
        "blocked_by": a.blocked_by or [],
        "origin_kind": a.origin_kind,
        "origin_fingerprint": a.origin_fingerprint or "default",
        "owner": None,
        "created_at": _now(),
        "updated_at": _now(),
    }
    _write_json_atomic(_task_path(task_id), task)
    print(task_id)
    return EXIT_OK


def cmd_checkout(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    if task["status"] in TERMINAL_STATUSES:
        print(f"conflict: task is terminal ({task['status']}) — 재시도 금지", file=sys.stderr)
        return EXIT_CONFLICT
    unresolved = _unresolved_blockers(task)
    if unresolved:
        print(f"blocked: 미해소 blocker {unresolved} — 체크아웃 거부", file=sys.stderr)
        return EXIT_BLOCKED
    verdict, holder = _acquire_lock(a.id, a.owner, pid=a.pid)
    if verdict == "conflict":
        who = (holder or {}).get("owner_id", "unknown")
        print(f"conflict(409): 살아있는 소유자 {who} — 재시도 금지, 다른 태스크로 이동", file=sys.stderr)
        return EXIT_CONFLICT
    task["status"] = "in_progress"
    task["owner"] = a.owner
    task["updated_at"] = _now()
    _write_json_atomic(_task_path(a.id), task)
    print(json.dumps({"checkout": verdict, "id": a.id, "owner": a.owner}, ensure_ascii=False))
    return EXIT_OK


def cmd_release(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    if not _release_lock(a.id, a.owner, force=a.force):
        holder = _read_owner(a.id)
        print(f"conflict(409): 락 소유자 불일치 (holder={(holder or {}).get('owner_id')})", file=sys.stderr)
        return EXIT_CONFLICT
    if task.get("owner") == a.owner or a.force:
        task["owner"] = None
        if task["status"] == "in_progress":
            task["status"] = "todo"
        task["updated_at"] = _now()
        _write_json_atomic(_task_path(a.id), task)
    print(f"released: {a.id}")
    return EXIT_OK


def cmd_set_status(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    task["status"] = a.status
    task["updated_at"] = _now()
    _write_json_atomic(_task_path(a.id), task)
    if a.status in TERMINAL_STATUSES:
        _release_lock(a.id, task.get("owner") or "", force=True)
        task["owner"] = None
        _write_json_atomic(_task_path(a.id), task)
    result = {"id": a.id, "status": a.status}
    if a.status == "done":
        result["unblocked"] = _find_unblocked_dependents(a.id)  # → javis_wakeup enqueue 대상
    print(json.dumps(result, ensure_ascii=False))
    return EXIT_OK


def cmd_ready(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    unresolved = _unresolved_blockers(task)
    print(json.dumps({"id": a.id, "ready": not unresolved, "unresolved": unresolved},
                     ensure_ascii=False))
    return EXIT_OK if not unresolved else EXIT_BLOCKED


def cmd_show(a):
    task = _read_task(a.id)
    if not task:
        print(f"not found: {a.id}", file=sys.stderr)
        return EXIT_NOTFOUND
    task["_lock_holder"] = _read_owner(a.id)
    print(json.dumps(task, ensure_ascii=False, indent=1))
    return EXIT_OK


def cmd_list(a):
    rows = _list_tasks()
    if a.open_only:
        rows = [t for t in rows if t.get("status") in OPEN_STATUSES]
    print(json.dumps(rows, ensure_ascii=False, indent=1))
    return EXIT_OK


def main(argv=None):
    p = argparse.ArgumentParser(description="원자적 태스크 체크아웃 (P0-1)")
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("create")
    c.add_argument("title")
    c.add_argument("--id")
    c.add_argument("--status", default="todo", choices=STATUSES)
    c.add_argument("--why", help="이 일이 왜 존재하는가(goal 체인)")
    c.add_argument("--goal")
    c.add_argument("--blocked-by", nargs="*", dest="blocked_by")
    c.add_argument("--origin-kind", default="manual")
    c.add_argument("--origin-fingerprint")
    c.set_defaults(fn=cmd_create)

    c = sub.add_parser("checkout")
    c.add_argument("id")
    c.add_argument("--owner", required=True, help="워커/세션 식별자")
    c.add_argument("--pid", type=int, help="락 생존 판정에 쓸 장수 프로세스 pid(기본: 호출자)")
    c.set_defaults(fn=cmd_checkout)

    c = sub.add_parser("release")
    c.add_argument("id")
    c.add_argument("--owner", required=True)
    c.add_argument("--force", action="store_true")
    c.set_defaults(fn=cmd_release)

    c = sub.add_parser("set-status")
    c.add_argument("id")
    c.add_argument("status", choices=STATUSES)
    c.set_defaults(fn=cmd_set_status)

    c = sub.add_parser("ready")
    c.add_argument("id")
    c.set_defaults(fn=cmd_ready)

    c = sub.add_parser("show")
    c.add_argument("id")
    c.set_defaults(fn=cmd_show)

    c = sub.add_parser("list")
    c.add_argument("--open-only", action="store_true")
    c.set_defaults(fn=cmd_list)

    a = p.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
