#!/usr/bin/env python3
"""
3단 사고 라우팅 엔진 (결정론 · 순수 표준 라이브러리)

책임: 요청을 "느린 사고(slow) / 숙고(deliberate) / 빠른 사고(fast)" 3단으로 판정한다.
워크플로우 선택은 마스터가 루트 폴더를 스캔해 판단한다.

판정 우선순위 (결정론):
    slow > deliberate > fast
    slow 토큰과 deliberate 토큰이 동시에 있으면 무거운 쪽(slow)이 이긴다.
    어떤 토큰도 없으면 fast — 애매한 경우의 격상(fast→deliberate/slow)은
    master의 LLM 판단 몫이다. 결정론 라우터는 과소발화(under-fire)가 안전하다.

3단 의미 (pack 규약 — 사고도구 계약):
    fast        초~분. master 직접 응답 (사전학습 + 스킬 + MCP).
    deliberate  분~1시간. 평가기준 선작성 + sub-agents 2-cycle 내부 검증.
    slow        시간 단위. 워커 위임 + agentic workflow + 외부 리뷰 라운드 + eval 게이트
                + 생존 계약(진행% 보고·체크포인트·watchdog·종료 게이트 기억 증류).

외부 의존성 없음:
    Python 3.8+ 기본 라이브러리만 사용. pip install 불필요.

이식성:
    자기 위치 옆의 route_triggers.json을 찾고, 없으면 구명 _slow_triggers.json을
    폴백으로 읽는다. 구 스키마(최상위 path/quality = slow 전용)도 자동 인식한다.
    파일 이름·위치가 무엇이든 그대로 작동.

사용법:
    # 기본 (옆의 route_triggers.json / _slow_triggers.json 자동 사용)
    python3 <이 파일> --request "박사급으로 분석해 줘"

    # 명시 지정
    python3 <이 파일> --triggers /path/to/route_triggers.json --request "..."

    # 결정론 자기검증 (preflight C17이 부트마다 호출)
    python3 <이 파일> --self-test

출력 (stdout, JSON):
    {"mode": "slow", "matched_token": "박사급으로", "group": "slow.quality"}
    {"mode": "deliberate", "matched_token": "교차검증해서", "group": "deliberate.quality"}
    {"mode": "fast", "matched_token": null, "group": null}

종료 코드: 0 정상 판정 · 1 self-test 실패 · 2 트리거 파일 없음 · 3 JSON 파싱 실패
"""

import argparse
import json
import sys
from pathlib import Path


# 스크립트 자기 위치 기반 기본 경로 (신명 우선, 구명 폴백)
SCRIPT_DIR = Path(__file__).resolve().parent
TRIGGER_FILENAMES = ("route_triggers.json", "_slow_triggers.json")

MODES = ("slow", "deliberate")  # 검사 순서 = 우선순위
GROUPS = ("path", "quality")

# 문장 경계: ASCII + 한국어 문서에 흔한 전각 부호
SENTENCE_BOUNDARY = set(".,!?\n") | set("。，！？；：…")


def default_triggers_path() -> Path:
    for name in TRIGGER_FILENAMES:
        p = SCRIPT_DIR / name
        if p.exists():
            return p
    return SCRIPT_DIR / TRIGGER_FILENAMES[0]


def _clean_tokens(value) -> list:
    """토큰 리스트 정제 — 비문자열·빈 토큰 제거 (빈 토큰은 find()==0으로
    전 요청을 강제 매칭시키는 무력화 버그가 되므로 결정론으로 차단한다)."""
    if not isinstance(value, list):
        return []
    return [t.strip() for t in value if isinstance(t, str) and t.strip()]


def normalize_schema(raw) -> dict:
    """스키마 정규화 — 어떤 입력에도 크래시 없이 {mode: {group: [tokens]}}로 환원.
    구 스키마(최상위 path/quality = slow 전용)·리스트형 모드(slow: [..] → path 취급)·
    비문자열/빈 토큰을 전부 관용 흡수한다."""
    if not isinstance(raw, dict):
        raw = {}
    if "slow" not in raw and "deliberate" not in raw:
        raw = {"slow": {"path": raw.get("path", []), "quality": raw.get("quality", [])}}
    out = {}
    for mode in MODES:
        g = raw.get(mode)
        if isinstance(g, list):
            g = {"path": g}
        if not isinstance(g, dict):
            g = {}
        out[mode] = {grp: _clean_tokens(g.get(grp, [])) for grp in GROUPS}
    return out


def _is_at_sentence_start(text: str, idx: int) -> bool:
    if idx == 0:
        return True
    j = idx - 1
    while j >= 0 and text[j] == " ":
        j -= 1
    if j < 0:
        return True
    return text[j] in SENTENCE_BOUNDARY


def _find_trigger(text: str, token: str) -> bool:
    token_lower = token.lower()
    start = 0
    while True:
        idx = text.find(token_lower, start)
        if idx < 0:
            return False
        if token_lower.startswith("/"):
            # 슬래시 커맨드: 위치는 자유롭되 직전이 시작/공백/문장경계여야 한다
            # — URL·파일경로 내부 부분일치(x.com/wf-docs) false positive 차단
            prev = text[idx - 1] if idx > 0 else ""
            if idx == 0 or prev in " \t" or prev in SENTENCE_BOUNDARY:
                return True
        else:
            if _is_at_sentence_start(text, idx):
                return True
        start = idx + 1


def route(request: str, triggers) -> dict:
    """
    라우터 본체. 트리거 토큰이 문장 경계에서 감지되면 해당 모드, 아니면 fast.
    모드 검사 순서 = 우선순위: slow 먼저, 그다음 deliberate.

    False positive 방지: 토큰은 문장 경계(시작, 마침표, 쉼표, 줄바꿈 직후)에서만 인정.
    본문 중간에 묻힌 토큰("업무 워크플로우로는 안 맞아")은 명령이 아닌 서술로 간주.
    슬래시 커맨드(/slow, /wf, /deliberate)는 직전이 시작/공백/경계일 때만 인정.
    """
    norm = request.lower().strip()
    triggers = normalize_schema(triggers)
    for mode in MODES:
        for group in GROUPS:
            for token in triggers[mode][group]:
                if _find_trigger(norm, token):
                    return {
                        "mode": mode,
                        "matched_token": token,
                        "group": "%s.%s" % (mode, group),
                    }
    return {"mode": "fast", "matched_token": None, "group": None}


def validate_config(raw) -> list:
    """배포 트리거 파일의 구조 검증 — 발견한 문제 목록을 돌려준다(없으면 빈 리스트).
    특정 토큰 문자열은 오너 주권(자유 편집)이라 핀하지 않고, 구조만 검증한다."""
    problems = []
    if not isinstance(raw, dict):
        return ["루트가 JSON 객체가 아님"]
    norm = normalize_schema(raw)
    if not any(norm["slow"][g] for g in GROUPS):
        problems.append("slow 트리거가 0개 — 느린 사고 진입 불가")
    for mode in MODES:
        g = raw.get(mode)
        src = g if isinstance(g, dict) else {}
        for grp in GROUPS:
            rawlist = src.get(grp, []) if isinstance(src, dict) else []
            if isinstance(rawlist, list):
                dropped = len(rawlist) - len(norm[mode][grp]) if isinstance(g, dict) else 0
                if dropped > 0:
                    problems.append(
                        "%s.%s에 무효 토큰 %d개(빈 문자열/비문자열) — 정리 필요"
                        % (mode, grp, dropped))
    return problems


def self_test() -> int:
    """결정론 자기검증 — ①판정 로직 배터리(합성 트리거) ②배포 트리거 파일 구조 검증.
    preflight C17이 부트마다 호출한다. 출력만이 사실이다."""
    synth = {
        "slow": {"path": ["워크플로우로", "/wf"], "quality": ["박사급으로"]},
        "deliberate": {"path": ["숙고해서", "/deliberate"], "quality": ["교차검증해서"]},
    }
    cases = [
        # (요청, 기대 모드, 설명)
        ("박사급으로 분석해 줘", "slow", "기본 slow"),
        ("숙고해서 답해줘", "deliberate", "기본 deliberate"),
        ("안녕, 오늘 어때", "fast", "기본 fast"),
        ("업무 워크플로우로는 안 맞아", "fast", "문장 중간 묻힘 → 서술 간주"),
        ("숙고해서, 워크플로우로 해줘", "slow", "동시 출현 시 slow 우선"),
        ("/wf 돌려", "slow", "슬래시 커맨드 행두"),
        ("이것 먼저. /deliberate 설계 검토", "deliberate", "슬래시 커맨드 문중(공백 뒤)"),
        ("http://x.com/wf-docs 읽어줘", "fast", "URL 내부 부분일치 차단"),
        ("경로 a/deliberate/b 확인", "fast", "파일경로 내부 부분일치 차단"),
        ("정리했다. 교차검증해서 다오", "deliberate", "ASCII 마침표 경계"),
        ("정리했다。교차검증해서 다오", "deliberate", "전각 마침표 경계"),
        ("그 박사급으로의 평판은", "fast", "문중 묻힘(앞에 '그 ')"),
    ]
    failures = []
    for req, want, why in cases:
        got = route(req, synth)["mode"]
        if got != want:
            failures.append("logic: %r → got %s, want %s (%s)" % (req, got, want, why))

    # 빈 토큰 무력화 가드: 빈/공백 토큰만 있으면 어떤 요청도 매칭되지 않아야 한다
    bad = {"slow": {"path": ["", "   "], "quality": [None, 7]}}
    if route("아무 말이나", bad)["mode"] != "fast":
        failures.append("logic: 빈/무효 토큰이 요청을 강제 매칭함 (무력화 버그)")
    # 구 스키마 호환
    legacy = {"path": ["워크플로우로"], "quality": []}
    if route("워크플로우로 해줘", legacy)["mode"] != "slow":
        failures.append("logic: 구 스키마(최상위 path/quality) 인식 실패")
    # 리스트형 모드 관용 정규화 (크래시 금지)
    if route("워크플로우로 해줘", {"slow": ["워크플로우로"]})["mode"] != "slow":
        failures.append("logic: 리스트형 slow 정규화 실패")
    # 완전 오염 입력도 크래시 없이 fast
    for garbage in (None, [], "x", {"slow": 3}, {"slow": {"path": "str"}}):
        try:
            if route("아무 말", garbage)["mode"] != "fast":
                failures.append("logic: 오염 스키마 %r가 fast가 아님" % (garbage,))
        except Exception as e:  # noqa: BLE001 — self-test는 모든 크래시를 결함으로 보고
            failures.append("logic: 오염 스키마 %r 크래시: %s" % (garbage, e))

    # 배포 트리거 파일 구조 검증
    tp = default_triggers_path()
    config_checked = False
    if tp.exists():
        try:
            raw = json.loads(tp.read_text(encoding="utf-8"))
            for p in validate_config(raw):
                failures.append("config(%s): %s" % (tp.name, p))
            config_checked = True
        except (OSError, ValueError) as e:
            failures.append("config: %s 읽기/파싱 실패: %s" % (tp, e))
    else:
        failures.append("config: 트리거 파일 없음 (%s)" % tp)

    print(json.dumps({
        "self_test": "ok" if not failures else "fail",
        "logic_cases": len(cases) + 8,
        "config_checked": str(tp) if config_checked else None,
        "failures": failures,
    }, ensure_ascii=False, indent=2))
    return 0 if not failures else 1


def main() -> int:
    p = argparse.ArgumentParser(
        description="3단 사고 라우팅 엔진 (결정론, 이식 가능)",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    p.add_argument(
        "--triggers",
        default=None,
        help="트리거 JSON 경로 (기본: 스크립트 옆 %s)" % " → ".join(TRIGGER_FILENAMES),
    )
    p.add_argument("--request", help="사용자 요청 문자열")
    p.add_argument("--self-test", action="store_true",
                   help="결정론 자기검증 실행 (0=통과 1=실패)")
    args = p.parse_args()

    if args.self_test:
        return self_test()
    if args.request is None:  # 빈 문자열("")은 유효 입력 — 토큰 없음 = fast 판정
        p.error("--request 또는 --self-test 가 필요하다")

    triggers_path = Path(args.triggers) if args.triggers else default_triggers_path()
    if not triggers_path.exists():
        print(
            json.dumps(
                {"error": "트리거 파일 없음: %s" % triggers_path},
                ensure_ascii=False,
            ),
            file=sys.stderr,
        )
        return 2

    try:
        with triggers_path.open("r", encoding="utf-8") as f:
            triggers = json.load(f)
    except json.JSONDecodeError as e:
        print(
            json.dumps(
                {"error": "JSON 파싱 실패: %s" % e},
                ensure_ascii=False,
            ),
            file=sys.stderr,
        )
        return 3

    result = route(args.request, triggers)
    print(json.dumps(result, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
