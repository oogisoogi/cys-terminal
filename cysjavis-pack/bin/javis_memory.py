#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_memory — 장기기억 증류 결정론 도구 (slow 종료 게이트의 기계 검증부).

절대지침 "slow 종료 게이트 = 기억 증류 의무"에서 결정론으로 환원 가능한 단계
(존재 검증·중복 검사·형식 검증·색인↔파일 정합·최근성 범위 검사)를 LLM 자연어
추론에서 분리해 수행한다. MEMORY.md 색인 손편집은 금지 — add가 원자적으로
파일 생성 + 색인 1줄 추가를 잠금(lock) 하에 수행한다(다중 노드 동시 쓰기 안전).

사용:
    python3 javis_memory.py add --type feedback --name <kebab-slug> \
        --desc "<한 줄 요약>" --body "<사실 본문>"        # 증류 1건 (원자적)
    python3 javis_memory.py verify [--json]               # 색인↔파일 정합 기계검증
    python3 javis_memory.py recent --minutes 1440 [--json] # 최근 증류 목록 (게이트 증거)
    python3 javis_memory.py --self-test                    # 결정론 자기검증 (preflight C18)

공통 옵션: --dir <memory 디렉터리> (기본: $CYS_PACK_DIR/memory 또는 ~/.cys/pack/memory)

종료 코드: 0 성공/정합 · 1 검증 실패 또는 self-test 실패 · 2 인자/입력 오류 · 3 잠금 실패
의존성: 파이썬 표준 라이브러리만 (네트워크·LLM 호출 없음).
"""

import argparse
import contextlib
import io
import json
import os
import re
import sys
import tempfile
import time

VALID_TYPES = ("user", "feedback", "project", "reference")
VALID_OUTCOMES = ("success", "failure", "neutral")  # ⑥ V사례 — feedback의 성공/실패 양면(과보수화 방지)
BLOAT_BYTES = 6144  # 단일 사실 메모리 비대 임계 — 초과 시 분할 후보(health·audit가 리포트만)
SLUG_RE = re.compile(r"^[a-z0-9][a-z0-9-]*$")
INDEX_FILE = "MEMORY.md"
INDEX_LINK_RE = re.compile(r"\]\(([^)\s]+\.md)\)")
HTML_COMMENT_RE = re.compile(r"<!--.*?-->", re.S)
FENCED_CODE_RE = re.compile(r"```.*?```", re.S)


def index_links(index_text):
    """색인 본문에서 링크 대상 추출 — HTML 주석·코드펜스 안의 예시는 색인이 아니다."""
    visible = FENCED_CODE_RE.sub("", HTML_COMMENT_RE.sub("", index_text))
    return [m.group(1) for m in INDEX_LINK_RE.finditer(visible)
            if "/" not in m.group(1) and m.group(1) != INDEX_FILE]


def default_memory_dir():
    """pack 위치 결정 — src/pack.rs pack_dir()의 폴백을 그대로 미러링한다."""
    for key in ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_JARVIS_DIR"):
        v = os.environ.get(key, "")
        if v:
            return os.path.join(v, "memory")
    return os.path.join(os.path.expanduser("~"), ".cys/pack", "memory")


class FileLock:
    """O_CREAT|O_EXCL 잠금파일 — posix·windows 공통 표준 라이브러리 구현.
    다중 노드가 동시에 색인을 갱신할 때의 append 유실을 차단한다."""

    def __init__(self, target, timeout=5.0, stale=30.0):
        self.path = target + ".lock"
        self.timeout = timeout
        self.stale = stale
        self.fd = None

    def __enter__(self):
        deadline = time.time() + self.timeout
        while True:
            try:
                self.fd = os.open(self.path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
                os.write(self.fd, str(os.getpid()).encode())
                return self
            except FileExistsError:
                try:  # 죽은 프로세스가 남긴 만료 잠금은 회수한다
                    if time.time() - os.path.getmtime(self.path) > self.stale:
                        os.unlink(self.path)
                        continue
                except OSError:
                    pass
                if time.time() > deadline:
                    raise TimeoutError("잠금 획득 실패(%.0fs): %s" % (self.timeout, self.path))
                time.sleep(0.05)

    def __exit__(self, *exc):
        if self.fd is not None:
            os.close(self.fd)
        try:
            os.unlink(self.path)
        except OSError:
            pass
        return False


def parse_frontmatter(text):
    """frontmatter에서 name/description/type 추출. 형식 불량이면 None 필드로 반환."""
    out = {"name": None, "description": None, "type": None}
    if not text.startswith("---"):
        return out
    end = text.find("\n---", 3)
    if end < 0:
        return out
    head = text[3:end]
    for line in head.splitlines():
        s = line.strip()
        if s.startswith("name:"):
            out["name"] = s[5:].strip()
        elif s.startswith("description:"):
            out["description"] = s[12:].strip()
        elif s.startswith("type:"):
            out["type"] = s[5:].strip()
    return out


def memory_files(mdir):
    try:
        names = sorted(os.listdir(mdir))
    except OSError:
        return []
    return [n for n in names
            if n.endswith(".md") and n != INDEX_FILE and not n.startswith(".")]


def cmd_add(mdir, args):
    if args.type not in VALID_TYPES:
        return fail(2, "type은 %s 중 하나" % "|".join(VALID_TYPES))
    if not SLUG_RE.match(args.name or ""):
        return fail(2, "name은 kebab-case 슬러그([a-z0-9-], 영숫자 시작)여야 한다: %r" % args.name)
    if not (args.desc or "").strip():
        return fail(2, "--desc(한 줄 요약)는 비울 수 없다")
    body = args.body
    if body is None and not sys.stdin.isatty():
        body = sys.stdin.read()
    if not (body or "").strip():
        return fail(2, "--body(사실 본문)는 비울 수 없다 (stdin도 가능)")

    os.makedirs(mdir, exist_ok=True)
    fname = "%s_%s.md" % (args.type, args.name)
    fpath = os.path.join(mdir, fname)
    index_path = os.path.join(mdir, INDEX_FILE)

    outcome = getattr(args, "outcome", None)
    if outcome and outcome not in VALID_OUTCOMES:
        return fail(2, "outcome은 %s 중 하나" % "|".join(VALID_OUTCOMES))
    meta_extra = "  outcome: %s\n" % outcome if outcome else ""
    content = (
        "---\n"
        "name: %s\n"
        "description: %s\n"
        "metadata:\n"
        "  type: %s\n"
        "%s"
        "---\n\n%s\n" % (args.name, args.desc.strip(), args.type, meta_extra, body.strip())
    )
    index_line = "- [%s](%s) — %s\n" % (args.name, fname, args.desc.strip())

    try:
        with FileLock(index_path):
            if os.path.exists(fpath):
                return fail(2, "중복 — 이미 존재: %s (갱신은 파일을 직접 수정)" % fname)
            # 색인에 같은 파일명이 이미 있으면 중복(파일만 지워진 잔재) — 거부
            existing_index = ""
            if os.path.isfile(index_path):
                existing_index = open(index_path, encoding="utf-8", errors="replace").read()
                if ("(%s)" % fname) in existing_index:
                    return fail(2, "색인에 %s 항목이 이미 있다 — verify로 정합부터 복구하라" % fname)
            # 원자적 파일 생성(O_EXCL) 후 색인 append
            fd = os.open(fpath, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                f.write(content)
            with open(index_path, "a", encoding="utf-8") as f:
                if existing_index and not existing_index.endswith("\n"):
                    f.write("\n")
                f.write(index_line)
    except TimeoutError as e:
        return fail(3, str(e))

    print(json.dumps({"added": fname, "index_line": index_line.strip()},
                     ensure_ascii=False))
    return 0


def collect_problems(mdir):
    """색인↔파일 정합·형식 문제 목록 (없으면 빈 리스트) — verify의 본체."""
    problems = []
    index_path = os.path.join(mdir, INDEX_FILE)
    if not os.path.isdir(mdir):
        return ["memory 디렉터리 없음: %s" % mdir]
    if not os.path.isfile(index_path):
        return ["%s 없음 — cys init-pack 또는 골격 복원 필요" % INDEX_FILE]

    files = memory_files(mdir)
    index_text = open(index_path, encoding="utf-8", errors="replace").read()
    linked = index_links(index_text)

    seen_names = {}
    for fn in files:
        text = open(os.path.join(mdir, fn), encoding="utf-8", errors="replace").read()
        fm = parse_frontmatter(text)
        if fm["name"] is None or fm["description"] is None:
            problems.append("%s: frontmatter(name/description) 불량" % fn)
        if fm["type"] not in VALID_TYPES:
            problems.append("%s: type 무효(%r) — %s 중 하나여야 함"
                            % (fn, fm["type"], "|".join(VALID_TYPES)))
        elif not fn.startswith(fm["type"] + "_"):
            problems.append("%s: 파일명이 '%s_' 접두가 아님 (type=%s)"
                            % (fn, fm["type"], fm["type"]))
        if fm["name"]:
            if fm["name"] in seen_names:
                problems.append("name 중복: %r (%s, %s)"
                                % (fm["name"], seen_names[fm["name"]], fn))
            seen_names[fm["name"]] = fn
        if fn not in linked:
            problems.append("색인 누락: %s가 %s에 없음" % (fn, INDEX_FILE))
    fileset = set(files)
    for target in linked:
        if target not in fileset:
            problems.append("dangling 색인: %s → 파일 없음" % target)
    # 중복 색인 줄
    for target in set(linked):
        if linked.count(target) > 1:
            problems.append("색인 중복 등재: %s (%d회)" % (target, linked.count(target)))
    return problems


def cmd_verify(mdir, as_json):
    problems = collect_problems(mdir)
    if as_json:
        print(json.dumps({"ok": not problems, "dir": mdir,
                          "files": len(memory_files(mdir)), "problems": problems},
                         ensure_ascii=False, indent=2))
    else:
        for p in problems:
            print("[FAIL] %s" % p)
        print("verify: %s — 파일 %d · 문제 %d (%s)"
              % ("OK" if not problems else "NOT OK",
                 len(memory_files(mdir)), len(problems), mdir))
        if problems:
            print("이 출력 외의 추론으로 정합을 선언하지 마라.")
    return 0 if not problems else 1


def cmd_recent(mdir, minutes, as_json):
    now = time.time()
    items = []
    for fn in memory_files(mdir):
        try:
            age_min = (now - os.path.getmtime(os.path.join(mdir, fn))) / 60.0
        except OSError:
            continue
        if age_min <= minutes:
            items.append({"file": fn, "age_minutes": round(age_min, 1)})
    items.sort(key=lambda x: x["age_minutes"])
    if as_json:
        print(json.dumps({"window_minutes": minutes, "count": len(items),
                          "items": items}, ensure_ascii=False, indent=2))
    else:
        for it in items:
            print("%-50s %6.1f분 전" % (it["file"], it["age_minutes"]))
        print("recent: %d건 (최근 %d분, %s)" % (len(items), minutes, mdir))
    return 0


def tokenize(s):
    """name·description 토큰화 — 한글·영숫자 토막 (중복 휴리스틱용)."""
    return set(re.findall(r"[a-z0-9가-힣]+", (s or "").lower()))


def memory_stats(mdir):
    """건강도 원자료 — 파일별 크기(B)·나이(일)·type 집계."""
    now = time.time()
    stats = []
    by_type = {}
    for fn in memory_files(mdir):
        p = os.path.join(mdir, fn)
        try:
            sz = os.path.getsize(p)
            age = (now - os.path.getmtime(p)) / 86400.0
        except OSError:
            continue
        fm = parse_frontmatter(open(p, encoding="utf-8", errors="replace").read())
        t = fm["type"] if fm["type"] in VALID_TYPES else "?"
        by_type[t] = by_type.get(t, 0) + 1
        stats.append({"file": fn, "bytes": sz, "age_days": round(age, 1), "type": t})
    return stats, by_type


def cmd_health(mdir, as_json):
    """건강도 대시보드 — 분석 전용(수정 0). 외부 /memory-health의 javis판.
    색인 정합(collect_problems)·비대·고령·type 분포를 한 화면에 요약한다."""
    problems = collect_problems(mdir)
    stats, by_type = memory_stats(mdir)
    total = len(stats)
    total_bytes = sum(s["bytes"] for s in stats)
    bloated = sorted([s for s in stats if s["bytes"] > BLOAT_BYTES], key=lambda s: -s["bytes"])
    oldest = sorted(stats, key=lambda s: -s["age_days"])[:5]
    ok = not problems and not bloated
    data = {
        "dir": mdir, "files": total, "by_type": by_type,
        "total_bytes": total_bytes,
        "avg_bytes": round(total_bytes / total) if total else 0,
        "bloated": [{"file": s["file"], "bytes": s["bytes"]} for s in bloated],
        "oldest": [{"file": s["file"], "age_days": s["age_days"]} for s in oldest],
        "integrity_problems": len(problems), "ok": ok,
    }
    if as_json:
        print(json.dumps(data, ensure_ascii=False, indent=2))
    else:
        print("memory health: %s" % ("OK" if ok else "주의"))
        print("  파일 %d · 총 %dB(평균 %dB) · type %s"
              % (total, total_bytes, data["avg_bytes"], by_type))
        print("  정합 문제 %d · 비대(>%dB) %d" % (len(problems), BLOAT_BYTES, len(bloated)))
        for s in bloated:
            print("    [비대] %-46s %dB" % (s["file"], s["bytes"]))
        if oldest:
            print("  최고령: %s (%.0f일)" % (oldest[0]["file"], oldest[0]["age_days"]))
    return 0


def duplicate_candidates(mdir, threshold):
    """같은 type 내 name+description 토큰 자카드 유사도 ≥ threshold 쌍 (중복 후보)."""
    recs = []
    for fn in memory_files(mdir):
        fm = parse_frontmatter(open(os.path.join(mdir, fn),
                                    encoding="utf-8", errors="replace").read())
        recs.append((fn, fm["type"], tokenize("%s %s" % (fm["name"] or "", fm["description"] or ""))))
    pairs = []
    for i in range(len(recs)):
        for j in range(i + 1, len(recs)):
            fa, ta, sa = recs[i]
            fb, tb, sb = recs[j]
            if ta != tb or not sa or not sb:
                continue
            union = len(sa | sb)
            sim = len(sa & sb) / union if union else 0.0
            if sim >= threshold:
                pairs.append({"a": fa, "b": fb, "type": ta, "similarity": round(sim, 2)})
    pairs.sort(key=lambda x: -x["similarity"])
    return pairs


def cmd_audit(mdir, threshold, as_json):
    """모순·중복 후보 분석 — 분석 전용. 병합·삭제는 사람/게이트 승인(자동수정 0).
    외부 /memory-consolidate(memory-auditor가 분석만)의 javis판."""
    dups = duplicate_candidates(mdir, threshold)
    stats, _ = memory_stats(mdir)
    bloated = [{"file": s["file"], "bytes": s["bytes"]} for s in stats if s["bytes"] > BLOAT_BYTES]
    data = {"dir": mdir, "threshold": threshold,
            "duplicate_candidates": dups, "bloated": bloated,
            "note": "분석 전용 — 병합·삭제는 사람/게이트 승인 후 (자동수정 0)"}
    if as_json:
        print(json.dumps(data, ensure_ascii=False, indent=2))
    else:
        print("memory audit (유사도≥%.2f): 중복후보 %d · 비대 %d"
              % (threshold, len(dups), len(bloated)))
        for d in dups:
            print("  [중복?] %.2f  %s ↔ %s (%s)" % (d["similarity"], d["a"], d["b"], d["type"]))
        print("  ※ 분석 전용 — 병합·삭제는 사람/게이트 승인 후")
    return 0


def fail(code, msg):
    print(json.dumps({"error": msg}, ensure_ascii=False), file=sys.stderr)
    return code


def self_test():
    """tempdir 라운드트립 — add→verify OK→고장 주입→verify가 잡는지까지 검증."""
    failures = []
    with tempfile.TemporaryDirectory(prefix="javis-memory-selftest-") as td:
        mdir = os.path.join(td, "memory")
        os.makedirs(mdir)
        # 골격에는 주석 예시·코드펜스가 들어있다 — 이를 색인으로 오인하면 안 된다(R2 실버그 박제)
        open(os.path.join(mdir, INDEX_FILE), "w", encoding="utf-8").write(
            "# MEMORY.md — self-test 골격\n\n"
            "```markdown\n- [예시](type_example.md) — 코드펜스 안 예시\n```\n\n"
            "## 색인\n\n<!-- - [제목](파일.md) — 핵심 한 줄 -->\n")
        if collect_problems(mdir):
            failures.append("주석/코드펜스 예시를 색인으로 오인: %s" % collect_problems(mdir))

        ns = argparse.Namespace(type="feedback", name="test-fact",
                                desc="자기검증용 사실", body="본문이다.")
        # 내부 호출의 stdout/stderr는 격리한다 — self-test의 JSON 출력만이 사실이다
        sink = io.StringIO()
        with contextlib.redirect_stdout(sink), contextlib.redirect_stderr(sink):
            add_rc = cmd_add(mdir, ns)
        if add_rc != 0:
            failures.append("add 실패")
        if collect_problems(mdir):
            failures.append("정상 상태인데 verify가 문제 보고: %s" % collect_problems(mdir))
        # 중복 add는 거부되어야 한다
        with contextlib.redirect_stdout(sink), contextlib.redirect_stderr(sink):
            dup_rc = cmd_add(mdir, ns)
        if dup_rc == 0:
            failures.append("중복 add가 거부되지 않음")
        # 고장 1: 파일 삭제(색인만 잔존) → dangling 검출
        os.unlink(os.path.join(mdir, "feedback_test-fact.md"))
        if not any("dangling" in p for p in collect_problems(mdir)):
            failures.append("dangling 색인을 검출하지 못함")
        # 고장 2: 색인 미등재 고아 파일 → 누락 검출
        open(os.path.join(mdir, "project_orphan.md"), "w", encoding="utf-8").write(
            "---\nname: orphan\ndescription: d\nmetadata:\n  type: project\n---\nx\n")
        if not any("색인 누락" in p for p in collect_problems(mdir)):
            failures.append("색인 누락 파일을 검출하지 못함")
        # 고장 3: frontmatter 불량 → 형식 검출
        open(os.path.join(mdir, "user_bad.md"), "w", encoding="utf-8").write("no frontmatter\n")
        if not any("frontmatter" in p for p in collect_problems(mdir)):
            failures.append("frontmatter 불량을 검출하지 못함")
        # recent: 방금 만든 파일이 잡혀야 한다
        recent_count = sum(1 for fn in memory_files(mdir)
                           if (time.time() - os.path.getmtime(os.path.join(mdir, fn))) / 60.0 <= 60)
        if recent_count < 1:
            failures.append("recent가 방금 만든 파일을 못 잡음")
        # ⑥ outcome 태그: add --outcome success → frontmatter 기록 + 형식 유지
        ns_oc = argparse.Namespace(type="feedback", name="oc-test",
                                   desc="성공 사례", body="V사례.", outcome="success")
        with contextlib.redirect_stdout(sink), contextlib.redirect_stderr(sink):
            oc_rc = cmd_add(mdir, ns_oc)
        oc_path = os.path.join(mdir, "feedback_oc-test.md")
        if oc_rc != 0 or "outcome: success" not in open(oc_path, encoding="utf-8").read():
            failures.append("--outcome 태그가 frontmatter에 기록되지 않음")
        # ② health: 비대 파일 검출
        open(os.path.join(mdir, "project_big.md"), "w", encoding="utf-8").write(
            "---\nname: big\ndescription: d\nmetadata:\n  type: project\n---\n"
            + "x" * (BLOAT_BYTES + 10) + "\n")
        if not any(s["bytes"] > BLOAT_BYTES for s in memory_stats(mdir)[0]):
            failures.append("health가 비대 파일을 못 잡음")
        # ② audit: 동일 description 두 파일 → 중복 후보
        for nm in ("dup-a", "dup-b"):
            open(os.path.join(mdir, "reference_%s.md" % nm), "w", encoding="utf-8").write(
                "---\nname: %s\ndescription: 환경 스캐닝 약신호 모니터링\n"
                "metadata:\n  type: reference\n---\nx\n" % nm)
        if not any({d["a"], d["b"]} == {"reference_dup-a.md", "reference_dup-b.md"}
                   for d in duplicate_candidates(mdir, 0.5)):
            failures.append("audit가 중복 후보를 못 잡음")
        # 잠금: 잔류 잠금파일이 없어야 한다
        if os.path.exists(os.path.join(mdir, INDEX_FILE + ".lock")):
            failures.append("잠금파일 잔류")

    print(json.dumps({"self_test": "ok" if not failures else "fail",
                      "failures": failures}, ensure_ascii=False, indent=2))
    return 0 if not failures else 1


def main():
    ap = argparse.ArgumentParser(description="장기기억 증류 결정론 도구")
    ap.add_argument("--dir", default=None, help="memory 디렉터리 (기본: pack/memory)")
    ap.add_argument("--self-test", action="store_true", help="결정론 자기검증")
    sub = ap.add_subparsers(dest="cmd")

    a = sub.add_parser("add", help="증류 1건 — 파일 생성 + 색인 1줄 (원자적·중복검사)")
    a.add_argument("--type", required=True, choices=VALID_TYPES)
    a.add_argument("--name", required=True, help="kebab-case 슬러그")
    a.add_argument("--desc", required=True, help="한 줄 요약 (색인에 실림)")
    a.add_argument("--body", default=None, help="사실 본문 (생략 시 stdin)")
    a.add_argument("--outcome", default=None, choices=VALID_OUTCOMES,
                   help="⑥ 성공/실패 양면 기록 (V=success·P=failure·neutral)")

    v = sub.add_parser("verify", help="색인↔파일 정합 기계검증 (0=정합 1=부정합)")
    v.add_argument("--json", action="store_true")

    r = sub.add_parser("recent", help="최근 N분 내 증류 목록 (게이트 증거)")
    r.add_argument("--minutes", type=int, default=1440)
    r.add_argument("--json", action="store_true")

    h = sub.add_parser("health", help="장기기억 건강도 대시보드 (분석 전용)")
    h.add_argument("--json", action="store_true")

    au = sub.add_parser("audit", help="모순·중복 후보 분석 (분석 전용 — 수정은 사람/게이트 승인)")
    au.add_argument("--threshold", type=float, default=0.5, help="중복 자카드 유사도 임계")
    au.add_argument("--json", action="store_true")

    args = ap.parse_args()
    if args.self_test:
        return self_test()
    mdir = args.dir or default_memory_dir()
    if args.cmd == "add":
        return cmd_add(mdir, args)
    if args.cmd == "verify":
        return cmd_verify(mdir, args.json)
    if args.cmd == "recent":
        return cmd_recent(mdir, args.minutes, args.json)
    if args.cmd == "health":
        return cmd_health(mdir, args.json)
    if args.cmd == "audit":
        return cmd_audit(mdir, args.threshold, args.json)
    ap.print_help()
    return 2


if __name__ == "__main__":
    sys.exit(main())
