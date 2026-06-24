#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_cleanroom — _research 흡수 보고서의 MPL/라이선스 클린룸 4원칙 헤더 검증·삽입.

오픈소스 흡수 연구(`_research/*_박사급_연구보고서.md`)가 라이선스(특히 MPL-2.0 파일단위
전염)를 어긴 코드복사로 오염되지 않도록 "코드복사 0 · 계약/패턴/산술만 클린룸 · 1차표준
직접 출처 · 복사 아님 명시" 4원칙을 기계 파싱 가능한 표준 헤더 블록으로 박제한다.
도메인-무관·결정론·순수 stdlib(추가 인프라 0·종량제 0).

서브커맨드:
    javis_cleanroom.py --self-test               # → exit 0, {"self_test":"ok"} (검증로직 자기검증)
    javis_cleanroom.py check  --root <_research>  # → {"ok":bool,"missing":[...],"broken":[...]}  exit 1 if any
    javis_cleanroom.py fix    --root <_research>  # → 누락 헤더 삽입(라이선스 자동탐지), {"fixed":[...]}

종료 코드: 0 = 정합/수리 완료, 1 = missing/broken 존재(check) 또는 self-test 실패.
"""

import argparse
import glob
import json
import os
import re
import sys

OPEN = "<!-- CLEANROOM-GUARDRAIL v1 -->"
CLOSE = "<!-- /CLEANROOM-GUARDRAIL -->"
# 4원칙 고정 키 — C41/C42가 존재 검사하는 기계 마커.
PRINCIPLE_KEYS = ("코드복사 0", "계약/패턴/산술", "1차표준", "복사 아님")
# 라이선스 SPDX 화이트리스트(추정 금지 — 미탐지 시 플레이스홀더 유지).
SPDX_TOKENS = ("MPL-2.0", "MIT", "AGPL", "GPL", "Apache-2.0", "BSD")
# 흡수 연구 보고서 파일 패턴.
REPORT_GLOB = "*_박사급_연구보고서.md"


def parse_block(text):
    """헤더 블록 1개 추출 → (found, ok, reason)."""
    if OPEN not in text:
        return (False, False, "no-header")
    if text.count(OPEN) != text.count(CLOSE) or CLOSE not in text:
        return (True, False, "marker-mismatch")
    block = text.split(OPEN, 1)[1].split(CLOSE, 1)[0]
    missing = [k for k in PRINCIPLE_KEYS if k not in block]
    if missing:
        return (True, False, "missing-keys:%s" % ",".join(missing))
    return (True, True, "ok")


def detect_license(text):
    """본문에서 라이선스 SPDX 토큰을 화이트리스트 매칭으로 추출(추정 금지)."""
    for tok in SPDX_TOKENS:
        if re.search(r"\b%s\b" % re.escape(tok), text):
            return tok
    return None


def render_header(spdx, lic_ref):
    spdx = spdx or "<LICENSE-SPDX>"
    return "\n".join([
        OPEN,
        "> **클린룸 가드레일(불변·기계검증 C42):**",
        "> ① **코드복사 0** — 대상 repo 소스 텍스트를 cys 트리에 복사하지 않는다"
        "(MPL-2.0 등 파일단위 전염 ↔ pack.rs `include_str!` 단일 서명 바이너리 충돌 회피).",
        "> ② **계약/패턴/산술만 클린룸** — enum·불변식·알고리즘·수치 규칙만 사양서 독립 재구현.",
        "> ③ **1차표준 직접 출처** — 2차 분석본 아닌 file:line/표준/URL 직접 근거.",
        "> ④ **복사 아님 명시** — 본 보고서의 모든 cys 매핑은 재구현이며 복제가 아님.",
        "> 대상 라이선스: `%s` (출처: `%s`)" % (spdx, lic_ref or "<LICENSE-PATH:LINE>"),
        CLOSE,
        "",
    ])


def insert_after_blockquote(text, header):
    """상단 > 인용블록 종료 직후·첫 '---' 앞에 삽입(본문 0라인 변경)."""
    m = re.search(r"\n---\n", text)  # 첫 구분선 = 헤더 영역 끝
    idx = m.start() + 1 if m else len(text)  # '---' 라인 직전(개행 보존)
    return text[:idx] + header + text[idx:]


def iter_reports(root):
    return sorted(glob.glob(os.path.join(root, REPORT_GLOB)))


def cmd_check(root):
    missing, broken = [], []
    for p in iter_reports(root):
        try:
            text = open(p, encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        found, ok, reason = parse_block(text)
        if not found:
            missing.append(p)
        elif not ok:
            broken.append(p)
    ok = not (missing or broken)
    print(json.dumps({"ok": ok, "missing": missing, "broken": broken}, ensure_ascii=False))
    return 0 if ok else 1


def cmd_fix(root):
    fixed, broken = [], []
    for p in iter_reports(root):
        try:
            text = open(p, encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        found, ok, _ = parse_block(text)
        if found and ok:
            continue  # 멱등 — 이미 정합인 헤더는 재삽입 0
        spdx = detect_license(text)
        if spdx is None:
            broken.append(p)  # 추정 삽입 금지 — 사람 보강 유도
        header = render_header(spdx, None)
        new = insert_after_blockquote(text, header)
        with open(p, "w", encoding="utf-8") as f:
            f.write(new)
        fixed.append(p)
    print(json.dumps({"fixed": fixed, "broken": broken}, ensure_ascii=False))
    return 0


def self_test():
    good = render_header("MPL-2.0", "x/LICENSE:1")
    f1, ok1, _ = parse_block(good)
    f2, ok2, _ = parse_block("no header here")
    f3, ok3, r3 = parse_block(OPEN + "\n> ① 코드복사 0\n" + CLOSE)  # keys missing
    # marker-mismatch: OPEN without CLOSE
    f4, ok4, r4 = parse_block(OPEN + "\n> 4원칙 ...\n")
    # insert idempotence pin
    body = "> intro\n\n---\n\nbody\n"
    once = insert_after_blockquote(body, good)
    failures = []
    if not (f1 and ok1):
        failures.append("render->parse round-trip")
    if f2 or ok2:
        failures.append("absent must be not-found")
    if ok3 or "missing-keys" not in r3:
        failures.append("partial must fail")
    if ok4 or r4 != "marker-mismatch":
        failures.append("unclosed marker must fail")
    if OPEN not in once or once.count(OPEN) != 1:
        failures.append("insert must place exactly one header")
    print(json.dumps({"self_test": "ok" if not failures else "fail",
                      "failures": failures}, ensure_ascii=False))
    return 1 if failures else 0


def main():
    ap = argparse.ArgumentParser(description="_research 클린룸 4원칙 헤더 검증·삽입")
    ap.add_argument("--self-test", action="store_true", help="검증 로직 자기검증(exit 0=ok)")
    sub = ap.add_subparsers(dest="cmd")
    pc = sub.add_parser("check", help="헤더 정합 검사")
    pc.add_argument("--root", required=True)
    pf = sub.add_parser("fix", help="누락 헤더 삽입(라이선스 자동탐지)")
    pf.add_argument("--root", required=True)
    args = ap.parse_args()

    if args.self_test:
        return self_test()
    if args.cmd == "check":
        return cmd_check(args.root)
    if args.cmd == "fix":
        return cmd_fix(args.root)
    ap.print_help()
    return 2


if __name__ == "__main__":
    sys.exit(main())
