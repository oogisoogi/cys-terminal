#!/usr/bin/env python3
"""오피스 디테일 v1.1 백엔드 단위 테스트 — stdlib unittest만 (신규 의존성 0).

대상: route_spool 4계열·귀속 규칙·날짜 롤오버·tail_spool 오프셋/truncate·칸반 스캔·
verdict md 파싱·히트 버킷 산술. 음성 케이스 포함(깨진 JSON·미지 type·spool 부재).
"""
import json
import os
import tempfile
import time
import unittest

BIN = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "bin")
if BIN not in __import__("sys").path:
    __import__("sys").path.insert(0, BIN)

import javis_hud_bridge as B  # noqa: E402
import javis_event as E       # noqa: E402


def make_world(roles=None):
    """지정 role→surface_ref 매핑으로 fleet가 채워진 World."""
    w = B.World()
    surfaces = []
    for role, ref in (roles or {}).items():
        surfaces.append({"role": role, "surface_ref": ref,
                         "surface_id": int(ref.split(":")[-1])})
    w.departments = [{"department": "본부", "surfaces": surfaces}]
    return w


class RouteSpoolProgress(unittest.TestCase):
    def test_progress_attributes_and_fx(self):
        w = make_world({"worker": "surface:12"})
        coal = B.Coalescer()
        entry = {"ts": time.time(), "type": "task_progress", "key": "surface:12",
                 "payload": {"task": "T1", "stage": "build", "pct": 40, "detail": "…"}}
        frames, poke = B.route_spool(entry, w, coal)
        self.assertFalse(poke)
        self.assertEqual(w.progress["surface:12"]["pct"], 40)
        self.assertEqual(w.progress["surface:12"]["stage"], "build")
        self.assertEqual([f for f in frames if f["kind"] == "progress"][0]["pct"], 40)

    def test_progress_non_increasing_pct_ignored(self):
        w = make_world({"worker": "surface:12"})
        coal = B.Coalescer()
        base = {"ts": time.time(), "type": "task_progress", "key": "surface:12"}
        B.route_spool({**base, "payload": {"task": "T1", "stage": "s", "pct": 50}}, w, coal)
        # 동일 (task,stage)에 pct 미증가 → 상태 유지·fx 없음
        frames, _ = B.route_spool(
            {**base, "payload": {"task": "T1", "stage": "s", "pct": 40}}, w, coal)
        self.assertEqual(w.progress["surface:12"]["pct"], 50)  # 40으로 후퇴 안 함
        self.assertEqual([f for f in frames if f.get("kind") == "progress"], [])

    def test_progress_increasing_pct_updates(self):
        w = make_world({"worker": "surface:12"})
        coal = B.Coalescer()
        base = {"ts": time.time(), "type": "task_progress", "key": "surface:12"}
        B.route_spool({**base, "payload": {"task": "T1", "stage": "s", "pct": 50}}, w, coal)
        frames, _ = B.route_spool(
            {**base, "payload": {"task": "T1", "stage": "s", "pct": 70}}, w, coal)
        self.assertEqual(w.progress["surface:12"]["pct"], 70)
        self.assertTrue(any(f.get("kind") == "progress" for f in frames))


class RouteSpoolRun(unittest.TestCase):
    def test_run_lifecycle_counters(self):
        w = make_world({"worker": "surface:5"})
        coal = B.Coalescer()
        k = "surface:5"
        for typ, pay in [("run.queued", {"agent": "worker", "task": "A"}),
                         ("run.queued", {"agent": "worker", "task": "B"}),
                         ("run.started", {"agent": "worker", "task": "A"}),
                         ("run.succeeded", {"agent": "worker", "task": "A", "summary": "ok"}),
                         ("run.failed", {"agent": "worker", "task": "B", "summary": "x"})]:
            B.route_spool({"ts": time.time(), "type": typ, "key": k, "payload": pay}, w, coal)
        r = w.run[k]
        self.assertEqual(r["queued"], 1)          # 2 enqueued − 1 started
        self.assertIsNone(r["active"])            # succeeded 후 해제
        self.assertEqual(r["done_today"], 1)
        self.assertEqual(r["failed_today"], 1)

    def test_run_started_sets_active(self):
        w = make_world({"worker": "surface:5"})
        coal = B.Coalescer()
        B.route_spool({"ts": time.time(), "type": "run.started", "key": "surface:5",
                       "payload": {"agent": "worker", "task": "Z"}}, w, coal)
        self.assertEqual(w.run["surface:5"]["active"]["task"], "Z")

    def test_run_date_rollover_resets_today(self):
        w = make_world({"worker": "surface:5"})
        coal = B.Coalescer()
        t0 = time.mktime((2026, 7, 11, 10, 0, 0, 0, 0, -1))
        B.route_spool({"ts": t0, "type": "run.succeeded", "key": "surface:5",
                       "payload": {"agent": "worker", "task": "A", "summary": "s"}},
                      w, coal, now=t0)
        self.assertEqual(w.run["surface:5"]["done_today"], 1)
        t1 = time.mktime((2026, 7, 12, 10, 0, 0, 0, 0, -1))  # 다음 날
        B.route_spool({"ts": t1, "type": "run.succeeded", "key": "surface:5",
                       "payload": {"agent": "worker", "task": "B", "summary": "s"}},
                      w, coal, now=t1)
        self.assertEqual(w.run["surface:5"]["done_today"], 1)  # 롤오버 후 리셋되어 다시 1


class Attribution(unittest.TestCase):
    def test_explicit_key_wins(self):
        w = make_world({"worker": "surface:1"})
        self.assertEqual(B.attribute_spool(
            {"key": "surface:99", "payload": {"agent": "worker"}}, w), "surface:99")

    def test_agent_unique_role_match(self):
        w = make_world({"worker": "surface:7"})
        self.assertEqual(B.attribute_spool({"payload": {"agent": "worker"}}, w), "surface:7")

    def test_agent_ambiguous_or_absent_is_none(self):
        w = make_world({"worker": "surface:7", "reviewer": "surface:8"})
        self.assertIsNone(B.attribute_spool({"payload": {"agent": "cso"}}, w))  # 매칭 role 없음
        self.assertIsNone(B.attribute_spool({"payload": {}}, w))               # agent 없음

    def test_blocked_keeps_null_key_when_unattributed(self):
        w = make_world({"worker": "surface:7"})
        coal = B.Coalescer()
        B.route_spool({"ts": time.time(), "type": "task.blocked",
                       "payload": {"task": "X", "blocked_by": ["Y"], "agent": "ghost"}},
                      w, coal)
        self.assertEqual(w.blocked[0]["task"], "X")
        self.assertIsNone(w.blocked[0]["key"])   # 미귀속 → key:null 로 남김


class RouteSpoolBlocked(unittest.TestCase):
    def test_block_then_unblock(self):
        w = make_world({"worker": "surface:3"})
        coal = B.Coalescer()
        B.route_spool({"ts": time.time(), "type": "task.blocked", "key": "surface:3",
                       "payload": {"task": "T", "blocked_by": ["dep1"]}}, w, coal)
        self.assertEqual(len(w.blocked), 1)
        B.route_spool({"ts": time.time(), "type": "task.blocked", "key": "surface:3",
                       "payload": {"task": "T", "blocked_by": ["dep1", "dep2"]}}, w, coal)
        self.assertEqual(len(w.blocked), 1)      # 동일 task 중복 갱신
        self.assertEqual(w.blocked[0]["blocked_by"], ["dep1", "dep2"])
        frames, _ = B.route_spool({"ts": time.time(), "type": "task.unblocked",
                                   "key": "surface:3", "payload": {"task": "T"}}, w, coal)
        self.assertEqual(w.blocked, [])
        self.assertTrue(any(f.get("kind") == "unblocked" for f in frames))

    def test_unknown_type_ignored(self):
        w = make_world()
        coal = B.Coalescer()
        frames, poke = B.route_spool(
            {"ts": time.time(), "type": "totally.bogus", "payload": {}}, w, coal)
        self.assertEqual(frames, [])
        self.assertFalse(poke)

    def test_backlog_suppresses_fx_keeps_state(self):
        w = make_world({"worker": "surface:2"})
        coal = B.Coalescer()
        old = time.time() - (B.BACKLOG_FX_SECS + 60)
        frames, _ = B.route_spool(
            {"ts": old, "type": "task.blocked", "key": "surface:2",
             "payload": {"task": "T", "blocked_by": ["d"]}}, w, coal, now=time.time())
        self.assertEqual(len(w.blocked), 1)      # 상태 반영
        self.assertEqual([f for f in frames if f.get("t") == "fx"], [])  # 연출 억제


class TailSpool(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.path = os.path.join(self.tmp, "evt_spool.jsonl")

    def _write(self, *entries):
        with open(self.path, "a") as f:
            for e in entries:
                f.write(json.dumps(e, ensure_ascii=False) + "\n")

    def test_absent_file_is_harmless(self):
        entries, off = B.tail_spool(os.path.join(self.tmp, "nope.jsonl"), 0)
        self.assertEqual(entries, [])
        self.assertEqual(off, 0)

    def test_reads_only_new_lines(self):
        self._write({"type": "a", "payload": {}}, {"type": "b", "payload": {}})
        entries, off = B.tail_spool(self.path, 0)
        self.assertEqual([e["type"] for e in entries], ["a", "b"])
        self._write({"type": "c", "payload": {}})
        entries2, off2 = B.tail_spool(self.path, off)
        self.assertEqual([e["type"] for e in entries2], ["c"])
        self.assertGreater(off2, off)

    def test_broken_json_line_skipped(self):
        with open(self.path, "w") as f:
            f.write('{"type":"ok","payload":{}}\n')
            f.write('this is not json\n')
            f.write('{"type":"ok2","payload":{}}\n')
        entries, _ = B.tail_spool(self.path, 0)
        self.assertEqual([e["type"] for e in entries], ["ok", "ok2"])

    def test_truncate_resets_offset(self):
        self._write({"type": "a", "payload": {}}, {"type": "b", "payload": {}})
        _, off = B.tail_spool(self.path, 0)
        with open(self.path, "w") as f:   # truncate + 새 줄
            f.write(json.dumps({"type": "fresh", "payload": {}}) + "\n")
        entries, _ = B.tail_spool(self.path, off)   # off > 새 파일 크기 → 0부터
        self.assertEqual([e["type"] for e in entries], ["fresh"])

    def test_partial_last_line_deferred(self):
        with open(self.path, "w") as f:
            f.write('{"type":"whole","payload":{}}\n{"type":"partial"')  # 개행 없음
        entries, off = B.tail_spool(self.path, 0)
        self.assertEqual([e["type"] for e in entries], ["whole"])
        with open(self.path, "a") as f:   # 나머지 완결
            f.write(',"payload":{}}\n')
        entries2, _ = B.tail_spool(self.path, off)
        self.assertEqual([e["type"] for e in entries2], ["partial"])


class EmitSpool(unittest.TestCase):
    def test_emit_spool_appends_entry(self):
        tmp = tempfile.mkdtemp()
        old = os.environ.get("HUD_STATE_DIR")
        os.environ["HUD_STATE_DIR"] = tmp
        try:
            E._spool_append("task.blocked", {"task": "T", "blocked_by": ["d"]}, "surface:4")
            with open(os.path.join(tmp, "evt_spool.jsonl")) as f:
                e = json.loads(f.readline())
            self.assertEqual(e["type"], "task.blocked")
            self.assertEqual(e["key"], "surface:4")
            self.assertEqual(e["payload"]["task"], "T")
            # tail_spool 이 emit 산출을 그대로 소비 가능한지 왕복 확인
            entries, _ = B.tail_spool(os.path.join(tmp, "evt_spool.jsonl"), 0)
            self.assertEqual(entries[0]["type"], "task.blocked")
        finally:
            if old is None:
                os.environ.pop("HUD_STATE_DIR", None)
            else:
                os.environ["HUD_STATE_DIR"] = old


class KanbanScan(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()

    def _task(self, name, **fields):
        with open(os.path.join(self.tmp, name), "w") as f:
            json.dump(fields, f)

    def test_status_normalization(self):
        self._task("a.json", id="a", title="A", status="in_progress", owner="worker",
                   blocked_by=[])
        self._task("b.json", id="b", title="B", status="done", owner=None, blocked_by=[])
        self._task("c.json", id="c", title="C", status="pending", owner=None, blocked_by=[])
        self._task("d.json", id="d", title="D", status="in_progress", owner=None,
                   blocked_by=["a"])   # 미완료+선행 → blocked
        kb = B.scan_kanban(self.tmp)
        by = {t["id"]: t["status"] for t in kb["tasks"]}
        self.assertEqual(by, {"a": "doing", "b": "done", "c": "todo", "d": "blocked"})

    def test_non_json_and_broken_skipped(self):
        self._task("ok.json", id="ok", title="OK", status="done", blocked_by=[])
        with open(os.path.join(self.tmp, "broken.json"), "w") as f:
            f.write("{ not json")
        os.mkdir(os.path.join(self.tmp, "sub.lock"))   # 디렉터리(실물 channels-c0.lock 형)
        kb = B.scan_kanban(self.tmp)
        self.assertEqual([t["id"] for t in kb["tasks"]], ["ok"])

    def test_absent_dir_harmless(self):
        kb = B.scan_kanban(os.path.join(self.tmp, "nope"))
        self.assertEqual(kb["tasks"], [])


class VerdictParse(unittest.TestCase):
    def test_markdown_equals_form(self):   # 실물: **verdict = ACCEPT**
        p = B.parse_verdict("**verdict = ACCEPT** (…)", "CSO_VERDICT_OPP21.md")
        self.assertEqual(p["verdict"], "ACCEPT")
        self.assertEqual(p["reviewer"], "CSO")
        self.assertEqual(p["target"], "OPP21")

    def test_json_colon_form(self):        # 실물: "verdict": "REVISE"
        p = B.parse_verdict('{\n  "verdict": "REVISE"\n}', "MACRT_T6B_VERDICT_codex.md")
        self.assertEqual(p["verdict"], "REVISE")
        self.assertEqual(p["reviewer"], "codex")
        self.assertEqual(p["target"], "MACRT_T6B")

    def test_reviewer_from_agy_filename(self):
        p = B.parse_verdict('"verdict": "ACCEPT"', "REVIEWER_AGY_VERDICT_CLI_PATH.md")
        self.assertEqual(p["reviewer"], "agy")
        self.assertEqual(p["target"], "CLI_PATH")

    def test_no_verdict_returns_none(self):
        self.assertIsNone(B.parse_verdict("no verdict token here", "X_VERDICT.md"))

    def test_scan_verdicts_keeps_latest(self):
        tmp = tempfile.mkdtemp()
        for i in range(12):
            fp = os.path.join(tmp, "CSO_VERDICT_P%02d.md" % i)
            with open(fp, "w") as f:
                f.write("verdict = ACCEPT")
            os.utime(fp, (1000 + i, 1000 + i))   # mtime 오름차순
        rv = B.scan_verdicts(tmp, keep=10)
        self.assertEqual(len(rv["items"]), 10)
        self.assertEqual(rv["items"][0]["target"], "P11")   # 최신 우선


class VerdictFx(unittest.TestCase):
    def test_fresh_verdict_emits_fx(self):
        now = time.time()
        items = [{"reviewer": "agy", "verdict": "ACCEPT", "target": "T", "ts": now - 5}]
        frames = B.verdict_fx(items, set(), now=now)
        self.assertEqual(len(frames), 1)
        self.assertEqual(frames[0]["kind"], "verdict")
        self.assertEqual(frames[0]["verdict"], "ACCEPT")

    def test_backlog_verdict_suppresses_fx_but_registers_seen(self):
        now = time.time()
        old_ts = now - (B.BACKLOG_FX_SECS + 60)
        items = [{"reviewer": "codex", "verdict": "REVISE", "target": "T", "ts": old_ts}]
        seen = set()
        frames = B.verdict_fx(items, seen, now=now)
        self.assertEqual(frames, [])                 # 과거 → fx 억제
        self.assertEqual(len(seen), 1)               # seen 등록은 됨 (이후 중복 방지)

    def test_restart_backlog_does_not_flood(self):
        # (재)기동 시 빈 seen에 10건 과거 verdict → fx 폭주 없어야 (결함 회귀 방지)
        now = time.time()
        items = [{"reviewer": "CSO", "verdict": "ACCEPT", "target": "P%d" % i,
                  "ts": now - (B.BACKLOG_FX_SECS + 100)} for i in range(10)]
        frames = B.verdict_fx(items, set(), now=now)
        self.assertEqual(frames, [])

    def test_seen_dedup_prevents_repeat(self):
        now = time.time()
        items = [{"reviewer": "agy", "verdict": "ACCEPT", "target": "T", "ts": now - 1}]
        seen = set()
        self.assertEqual(len(B.verdict_fx(items, seen, now=now)), 1)
        self.assertEqual(B.verdict_fx(items, seen, now=now), [])   # 동일 항목 재방출 없음

    def test_mixed_fresh_and_old(self):
        now = time.time()
        items = [{"reviewer": "agy", "verdict": "ACCEPT", "target": "new", "ts": now - 3},
                 {"reviewer": "agy", "verdict": "BLOCK", "target": "old",
                  "ts": now - (B.BACKLOG_FX_SECS + 30)}]
        frames = B.verdict_fx(items, set(), now=now)
        self.assertEqual([f["target"] for f in frames], ["new"])   # 신선만


class HeatBuckets(unittest.TestCase):
    def test_ratio_arithmetic(self):
        w = make_world({"worker": "surface:1"})
        # active 판정을 강제하기 위해 최근 hook 주입
        now = time.time()
        w.hooks[1].append(now)
        for _ in range(3):
            w.accumulate_heat(now)
        hour = time.localtime(now).tm_hour
        acc = w.heat_acc["surface:1"]
        self.assertEqual(acc["total"][hour], 3)
        self.assertEqual(acc["active"][hour], 3)      # 매 틱 active
        board = w.board_snapshot(now)
        self.assertEqual(board["heat"]["surface:1"][hour], 1.0)

    def test_inactive_ratio_zero(self):
        w = make_world({"worker": "surface:1"})
        w.departments[0]["surfaces"][0]["idle_secs"] = 5000   # sleeping
        now = time.time()
        w.accumulate_heat(now)
        hour = time.localtime(now).tm_hour
        self.assertEqual(w.board_snapshot(now)["heat"]["surface:1"][hour], 0.0)


class CostBestEffort(unittest.TestCase):
    def test_absent_db_returns_null(self):
        out = B.read_cost_today("/nonexistent/path/x.db")
        self.assertEqual(out, {"usd": None, "tokens": None})

    def test_schema_without_cost_columns_returns_null(self):
        import sqlite3
        tmp = tempfile.mkdtemp()
        db = os.path.join(tmp, "t.db")
        con = sqlite3.connect(db)
        con.execute("CREATE TABLE lines(id INTEGER, ts REAL, line TEXT)")
        con.execute("INSERT INTO lines VALUES (1, ?, 'x')", (time.time(),))
        con.commit(); con.close()
        out = B.read_cost_today(db)     # 비용/토큰 컬럼 없음 → null (죽지 않음)
        self.assertEqual(out, {"usd": None, "tokens": None})


class TopFrame(unittest.TestCase):
    def test_patch_top_only_on_change(self):
        w = make_world({"worker": "surface:1"})
        coal = B.Coalescer()
        B.route_spool({"ts": time.time(), "type": "task.blocked", "key": "surface:1",
                       "payload": {"task": "T", "blocked_by": ["d"]}}, w, coal)
        fr = w.top_frame("blocked")
        self.assertIsNotNone(fr)
        self.assertEqual(fr["field"], "blocked")
        self.assertIsNone(w.top_frame("blocked"))   # 변화 없음 → None


class SnapshotShape(unittest.TestCase):
    def test_contract_fields_present(self):
        w = make_world({"worker": "surface:1"})
        snap = w.snapshot()
        for field in ("blocked", "kanban", "review", "board"):
            self.assertIn(field, snap)
        self.assertIn("heat", snap["board"])
        self.assertIn("cost_today", snap["board"])
        node = snap["departments"][0]["nodes"][0]
        self.assertIn("progress", node)
        self.assertIn("run", node)
        self.assertEqual(snap["v"], 1)   # 계약 버전 불변


if __name__ == "__main__":
    unittest.main()
