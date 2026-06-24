#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_select — 채점식 provider/도구 선택 엔진 (도메인-무관·결정론).

"처음 사용 가능한 provider" 대신 가중 다차원 적합도로 후보를 랭킹하고, **가용성은
점수가 아니라 하드 게이트**(deny-by-default: 키 없으면 후보에서 제외·setup_offer로 안내)로
다룬다. cys 철학: 미디어부서 등 에이전트가 과제마다 *자율 선택*하되, 선택의 근거가
설명가능해야 한다. 카탈로그(capability→providers)는 외부 JSON 데이터다(이 엔진은 그 위에서
랭킹만 한다 — 영상 provider 카탈로그는 영상 v2가 공급).

cys 제약 정합:
- **무점수 채널 오염 금지**: 여기서의 fit(0~1)은 *라우팅 적합도*이지 리뷰어 품질 verdict가
  아니다. 절대 4자수렴 게이트에 품질 점수로 먹이지 않는다(REVIEWER_VERDICT_CONTRACT §1과 무관 층위).
- **Max전용·무료우선**: cost_tier {free|low|high}. free(로컬·스톡·Piper)가 충분하면 우선.
- **deny-by-default**: key_env 미설정 + 비-free = 가용 불가(랭킹 제외·setup_offer).
- **로컬 런타임 준비성(W0-4)**: free+local은 probe(bin·module·path)가 실제 설치/캐시됐을 때만 가용 —
  미준비면 deny+설치 안내(무음실패 차단). 'free+local=항상 가용'의 위험한 가정을 닫는다(필요조건 floor).
- **사용자 선호 우선**: prefer가 가용 후보면 강제 1위(AGENT_GUIDE: preference > availability > score).

사용:
    python3 javis_select.py rank --catalog <C.json> --capability <CAP> \
        [--intent "..."] [--style a,b] [--prefer ID] [--free-first] [--locked ID] [--json]
    python3 javis_select.py menu --catalog <C.json> [--json]   # capability별 가용/미가용 N-of-M 메뉴
    python3 javis_select.py --self-test

종료 코드: 0 성공(1위 결정) · 1 가용 후보 없음(전부 키 미설정 등) · 2 인자/입력 오류 · 3 — (미사용)
의존성: 파이썬 표준 라이브러리만 (네트워크·LLM 호출 없음·점수를 게이트에 먹이지 않음).
"""

import argparse
import io
import contextlib
import importlib.util
import json
import os
import re
import shutil
import sys
import tempfile

# 가중치 — 합 1.0. task_fit 최우선, cost(Max전용 무료우선)·quality 동률 차순.
WEIGHTS = {"task_fit": 0.35, "quality": 0.20, "cost": 0.20,
           "reliability": 0.10, "control": 0.08, "continuity": 0.07}
# 카탈로그 실제 어휘(excellent|good|fair)에 정합 — 'fair' 누락 시 fair provider가 조용히 default
# 점수로 떨어지던 잠복 버그 교정(W1-3 verify가 검출). basic·premium은 후방호환 별칭으로 유지.
QUALITY_SCORE = {"excellent": 0.95, "good": 0.7, "fair": 0.45, "basic": 0.4, "premium": 0.95}
COST_SCORE = {"free": 1.0, "low": 0.7, "high": 0.4}
RUNTIME_REL = {"local": 0.95, "stock": 0.9, "api": 0.7, "local_gpu": 0.85}
VALID_COST = tuple(COST_SCORE)
VALID_QUALITY = tuple(QUALITY_SCORE)

# ── 프로바이더 계약(W1-3 verify) — 카탈로그 레코드를 인터페이스-적합 플러그형으로 ──
PROVIDER_KEYS = ("id", "best_for", "quality_tier", "cost_tier", "key_env", "runtime", "supports", "probe")
PROVIDER_REQUIRED = ("id", "runtime", "cost_tier")
PROBE_KEYS = ("bin", "bins", "any_bin", "module", "modules", "any_module", "path", "paths")
PROBE_STR_KEYS = ("bin", "module", "path")
PROBE_LIST_KEYS = ("bins", "any_bin", "modules", "any_module", "paths")

# 의미 동의어 군 — "cinematic"과 "film"이 키워드는 달라도 매치(lib/scoring.py 발상 이식).
SYNONYMS = [
    {"cinematic", "film", "movie", "trailer", "dramatic", "epic"},
    {"explainer", "educational", "tutorial", "teaching", "lesson"},
    {"social", "tiktok", "reels", "shorts", "viral"},
    {"animation", "animated", "motion", "kinetic"},
    {"realistic", "photorealistic", "lifelike"},
    {"stock", "footage", "b-roll", "broll", "archive"},
    {"avatar", "presenter", "talking-head", "spokesperson"},
    {"voice", "voiceover", "narration", "speech", "tts"},
    {"music", "soundtrack", "score", "ambient"},
]
TOKEN_RE = re.compile(r"[a-z0-9가-힣][a-z0-9가-힣+._-]*")


def _tok(s):
    return set(TOKEN_RE.findall((s or "").lower()))


def _expand(words):
    out = set(words)
    for cl in SYNONYMS:
        if out & cl:
            out |= cl
    return out


def _overlap(a, b):
    """overlap coefficient |A∩B|/min(|A|,|B|) — Jaccard가 풍부한 best_for를 과벌하는 문제 회피."""
    if not a or not b:
        return 0.0
    m = min(len(a), len(b))
    return len(a & b) / m if m else 0.0


def _bin_ready(name):
    return shutil.which(name) is not None


def _module_ready(name):
    try:
        return importlib.util.find_spec(name) is not None
    except (ImportError, ValueError):
        return False


def runtime_ready(provider):
    """로컬 런타임 준비성 게이트(W0-4) — 'free+local'이 '설치됨+캐시됨'을 의미하게 한다.
    probe 미선언이면 ready(레거시 보존). 선언 시 bin(PATH)·module(import)·path(존재)를 실측한다.
    필요조건 floor(necessary-not-sufficient: torch가 있어도 모델 가중치는 별도) — 미준비를 자신 있게
    선택해 런타임에 무음실패하는 것을 차단한다. 반환 (ready, reason). 모든 선언 키가 통과해야 ready."""
    probe = provider.get("probe")
    if not probe:
        return True, ""
    missing = []
    b = probe.get("bin")
    if b and not _bin_ready(b):
        missing.append("바이너리 '%s' PATH에 없음" % b)
    for b in probe.get("bins", []) or []:
        if not _bin_ready(b):
            missing.append("바이너리 '%s' 없음" % b)
    anyb = probe.get("any_bin")
    if anyb and not any(_bin_ready(x) for x in anyb):
        missing.append("바이너리 %s 중 하나도 없음" % "|".join(anyb))
    m = probe.get("module")
    if m and not _module_ready(m):
        missing.append("파이썬 모듈 '%s' 미설치" % m)
    for m in probe.get("modules", []) or []:
        if not _module_ready(m):
            missing.append("파이썬 모듈 '%s' 미설치" % m)
    anym = probe.get("any_module")
    if anym and not any(_module_ready(x) for x in anym):
        missing.append("파이썬 모듈 %s 중 하나도 없음" % "|".join(anym))
    p = probe.get("path")
    if p and not os.path.exists(os.path.expanduser(p)):
        missing.append("경로 '%s' 없음(모델 미캐시)" % p)
    for p in probe.get("paths", []) or []:
        if not os.path.exists(os.path.expanduser(p)):
            missing.append("경로 '%s' 없음" % p)
    if missing:
        return False, "런타임 미준비: " + "; ".join(missing)
    return True, ""


def key_available(provider):
    """deny-by-default 키 게이트. key_env 없으면 통과(무키 free), 있으면 env 설정 필요."""
    key_env = provider.get("key_env")
    if not key_env:
        return True  # 키 불필요(로컬·스톡·무료)
    return bool(os.environ.get(key_env))


def available(provider):
    """가용성 = 키 게이트 ∧ 로컬 런타임 준비성. free+local도 미설치/미캐시면 미가용(무음실패 차단)."""
    return key_available(provider) and runtime_ready(provider)[0]


def score_provider(provider, ctx, free_first):
    """단일 provider의 라우팅 적합도(0~1) + 차원별 근거."""
    best = _expand(_tok(" ".join(provider.get("best_for", []))))
    intent = _expand(_tok(ctx.get("intent", "")) | set(x.lower() for x in ctx.get("style", [])))
    task_fit = _overlap(intent, best) if intent else 0.4

    quality = QUALITY_SCORE.get(provider.get("quality_tier", "good"), 0.6)
    cost_tier = provider.get("cost_tier", "high")
    cost = COST_SCORE.get(cost_tier, 0.4)
    if free_first and cost_tier == "free":
        cost = 1.0  # 무료우선 모드에서 free에 만점
    reliability = RUNTIME_REL.get(provider.get("runtime", "api"), 0.6)
    control = min(1.0, len(provider.get("supports", {})) / 5.0 + 0.2)
    locked = ctx.get("locked")
    continuity = 0.9 if locked and provider.get("id") == locked else (0.5 if not locked else 0.4)

    dims = {"task_fit": round(task_fit, 3), "quality": round(quality, 3),
            "cost": round(cost, 3), "reliability": round(reliability, 3),
            "control": round(control, 3), "continuity": round(continuity, 3)}
    fit = sum(dims[k] * w for k, w in WEIGHTS.items())
    top = sorted(dims.items(), key=lambda kv: -kv[1] * WEIGHTS[kv[0]])[:2]
    why = ", ".join("%s=%.2f" % (k, v) for k, v in top)
    if free_first and cost_tier == "free":
        why = "무료우선·" + why
    return round(fit, 3), dims, why


def load_catalog(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def rank(catalog, capability, ctx, free_first):
    """capability의 가용 provider를 랭킹. 반환 (ranked, unavailable, forced_pref|None)."""
    providers = (catalog.get("capabilities", {}) or {}).get(capability, [])
    ranked, unavailable = [], []
    for p in providers:
        if available(p):
            fit, dims, why = score_provider(p, ctx, free_first)
            ranked.append({"id": p.get("id"), "provider": p.get("provider"),
                           "fit": fit, "dims": dims, "why": why,
                           "cost_tier": p.get("cost_tier"), "quality_tier": p.get("quality_tier")})
        elif not key_available(p):
            unavailable.append({"id": p.get("id"), "reason": "key", "key_env": p.get("key_env"),
                                "setup": "키 %s 설정 시 가용" % p.get("key_env")})
        else:
            _, why = runtime_ready(p)  # 키는 통과했으나 로컬 런타임 미준비(W0-4)
            unavailable.append({"id": p.get("id"), "reason": "runtime",
                                "setup": why + " — 설치/캐시 시 가용"})
    ranked.sort(key=lambda r: -r["fit"])
    forced = None
    prefer = ctx.get("prefer")
    if prefer:
        for i, r in enumerate(ranked):
            if r["id"] == prefer:
                forced = r
                ranked.insert(0, ranked.pop(i))  # 선호를 1위로
                break
    return ranked, unavailable, forced


def cmd_rank(catalog, args):
    if args.capability not in (catalog.get("capabilities", {}) or {}):
        return fail(2, "capability 없음: %s (가용: %s)"
                    % (args.capability, ",".join(catalog.get("capabilities", {}))))
    ctx = {"intent": args.intent or "",
           "style": [s for s in (args.style or "").split(",") if s.strip()],
           "prefer": args.prefer, "locked": args.locked}
    ranked, unavailable, forced = rank(catalog, args.capability, ctx, args.free_first)
    chosen = ranked[0] if ranked else None
    note = []
    if forced:
        note.append("사용자 선호(%s)를 1위로 강제(가용 확인됨)" % forced["id"])
    if not ranked:
        note.append("가용 후보 0 — 전부 키 미설정/비가용. setup_offer 참조")
    out = {"capability": args.capability, "free_first": args.free_first,
           "chosen": chosen, "ranking": ranked, "unavailable": unavailable, "note": note}
    if args.json:
        print(json.dumps(out, ensure_ascii=False, indent=2))
    else:
        print("select %s: %s" % (args.capability,
              ("→ %s (fit %.2f · %s)" % (chosen["id"], chosen["fit"], chosen["why"])) if chosen else "가용 후보 없음"))
        for r in ranked[1:]:
            print("   %-22s fit %.2f · %s" % (r["id"], r["fit"], r["why"]))
        for u in unavailable:
            print("   [미가용] %-18s %s" % (u["id"], u["setup"]))
        for n in note:
            print("   · %s" % n)
    return 0 if chosen else 1


def cmd_menu(catalog, as_json):
    """capability별 N-of-M 가용 메뉴(정직한 능력봉투 — 하드코딩 금지·카탈로그 파생)."""
    menu = {}
    for cap, provs in (catalog.get("capabilities", {}) or {}).items():
        avail = [p["id"] for p in provs if available(p)]
        unav = []
        for p in provs:
            if available(p):
                continue
            if not key_available(p):
                unav.append({"id": p["id"], "reason": "key", "hint": "키 %s" % p.get("key_env")})
            else:
                _, why = runtime_ready(p)  # 로컬 런타임 미준비(W0-4)
                unav.append({"id": p["id"], "reason": "runtime", "hint": why})
        menu[cap] = {"configured": len(avail), "total": len(provs),
                     "available": avail, "unavailable": unav}
    if as_json:
        print(json.dumps(menu, ensure_ascii=False, indent=2))
    else:
        for cap, m in sorted(menu.items()):
            print("%-22s %d/%d  [%s]" % (cap, m["configured"], m["total"], ", ".join(m["available"])))
            for u in m["unavailable"]:
                print("      ↳ %s (%s)" % (u["id"], u["hint"]))
    return 0


def verify_catalog(catalog):
    """프로바이더 계약 적합성 린트 → (errors, warnings). 카탈로그 레코드가 javis_select가 기대하는
    인터페이스에 부합하는지 검증한다(OpenCut StickerProvider register-bug 교훈: 계약 미검증은 무음
    오동작). errors=차단(스코어 무음 default·미지 키 등), warnings=권고(준비성 게이트 부재 등)."""
    errors, warnings = [], []
    caps = catalog.get("capabilities")
    if not isinstance(caps, dict) or not caps:
        return (["capabilities가 비어있지 않은 객체가 아님"], [])
    seen_ids = {}
    for cap, provs in caps.items():
        if not isinstance(provs, list):
            errors.append("capability '%s' 값이 배열 아님" % cap)
            continue
        for i, p in enumerate(provs):
            if not isinstance(p, dict):
                errors.append("%s[%d] 객체 아님" % (cap, i))
                continue
            pid = p.get("id")
            w = "%s(%s)" % (cap, pid if pid else "?")
            for k in p:
                if k not in PROVIDER_KEYS:
                    errors.append("%s 미지 키 %r — %s" % (w, k, "|".join(PROVIDER_KEYS)))
            for k in PROVIDER_REQUIRED:
                if k not in p:
                    errors.append("%s 필수 키 누락: %s" % (w, k))
            if not (isinstance(pid, str) and pid.strip()):
                errors.append("%s id 비어있지 않은 문자열 필요" % w)
            else:
                seen_ids.setdefault(pid, []).append(cap)
            rt = p.get("runtime")
            if rt is not None and rt not in RUNTIME_REL:
                errors.append("%s runtime 무효(%r) — %s" % (w, rt, "|".join(RUNTIME_REL)))
            ct = p.get("cost_tier")
            if ct is not None and ct not in COST_SCORE:
                errors.append("%s cost_tier 무효(%r) — %s" % (w, ct, "|".join(COST_SCORE)))
            if "quality_tier" in p and p["quality_tier"] not in QUALITY_SCORE:
                errors.append("%s quality_tier 무효(%r) — %s (미인식=점수 무음 default)"
                              % (w, p["quality_tier"], "|".join(QUALITY_SCORE)))
            ke = p.get("key_env")
            if ke is not None and not (isinstance(ke, str) and ke.strip()):
                errors.append("%s key_env는 null 또는 비어있지 않은 문자열" % w)
            if "best_for" in p and not isinstance(p["best_for"], list):
                errors.append("%s best_for 배열 아님" % w)
            if "supports" in p and not isinstance(p["supports"], dict):
                errors.append("%s supports 객체 아님" % w)
            probe = p.get("probe")
            if probe is not None:
                if not isinstance(probe, dict) or not probe:
                    errors.append("%s probe 비어있지 않은 객체 필요" % w)
                else:
                    for k in probe:
                        if k not in PROBE_KEYS:
                            errors.append("%s probe 미지 키 %r — %s" % (w, k, "|".join(PROBE_KEYS)))
                    for k in PROBE_STR_KEYS:
                        if k in probe and not (isinstance(probe[k], str) and probe[k].strip()):
                            errors.append("%s probe.%s 비어있지 않은 문자열 필요" % (w, k))
                    for k in PROBE_LIST_KEYS:
                        if k in probe and not (isinstance(probe[k], list) and probe[k]
                                               and all(isinstance(x, str) and x.strip() for x in probe[k])):
                            errors.append("%s probe.%s 비어있지 않은 문자열 배열 필요" % (w, k))
            # W0-4 권고: 무키 local/local_gpu 인데 probe 없으면 준비성 게이트 부재(미설치도 가용 선택 위험)
            if rt in ("local", "local_gpu") and not p.get("key_env") and not probe:
                warnings.append("%s 무키 %s 인데 probe 없음 — 준비성 게이트 부재" % (w, rt))
    for pid, cw in seen_ids.items():
        if len(cw) > 1:
            warnings.append("id '%s' 가 여러 capability에 중복: %s" % (pid, ", ".join(cw)))
    return errors, warnings


def cmd_verify(catalog, as_json):
    errors, warnings = verify_catalog(catalog)
    ok = not errors
    if as_json:
        print(json.dumps({"ok": ok, "errors": errors, "warnings": warnings},
                         ensure_ascii=False, indent=2))
    else:
        for e in errors:
            print("[ERROR] %s" % e)
        for wn in warnings:
            print("[WARN] %s" % wn)
        print("catalog verify: %s — %d errors, %d warnings"
              % ("OK" if ok else "REJECT", len(errors), len(warnings)))
        if not ok:
            print("이 출력 외 추론으로 카탈로그 정합을 선언하지 마라.")
    return 0 if ok else 1


def fail(code, msg):
    print(json.dumps({"error": msg}, ensure_ascii=False), file=sys.stderr)
    return code


def self_test():
    failures = []
    cat = {"capabilities": {"video_generation": [
        {"id": "fal-kling", "provider": "fal", "key_env": "FAL_KEY", "cost_tier": "high",
         "quality_tier": "good", "runtime": "api", "best_for": ["b-roll", "image-to-video", "cheap"]},
        {"id": "fal-seedance", "provider": "fal", "key_env": "FAL_KEY", "cost_tier": "high",
         "quality_tier": "premium", "runtime": "api", "best_for": ["cinematic", "trailer", "premium"]},
        {"id": "stock-pexels", "provider": "pexels", "cost_tier": "free",
         "quality_tier": "good", "runtime": "stock", "best_for": ["b-roll", "stock", "footage"]},
        {"id": "wan-local", "provider": "wan", "key_env": "WAN_LOCAL", "cost_tier": "free",
         "quality_tier": "good", "runtime": "local_gpu", "best_for": ["b-roll", "free"]},
    ]}}
    sink = io.StringIO()

    def _rank(ctx, free_first=False):
        return rank(cat, "video_generation", ctx, free_first)

    # 1) deny-by-default: FAL_KEY 미설정 환경에선 fal-* 제외, 키 불요 stock만 가용
    saved = os.environ.pop("FAL_KEY", None)
    try:
        ranked, unav, _ = _rank({"intent": "b-roll"})
        avail_ids = {r["id"] for r in ranked}
        if "fal-kling" in avail_ids or "fal-seedance" in avail_ids:
            failures.append("키 없는 fal-*가 가용으로 랭킹됨(deny-by-default 위반)")
        if "stock-pexels" not in avail_ids:
            failures.append("키 불요 stock이 가용에서 빠짐")
        if not any(u["id"] == "fal-kling" for u in unav):
            failures.append("미가용 fal-kling이 setup_offer에 없음")
        # 2) 가용 후보 0 → exit 1
        empty = {"capabilities": {"x": [{"id": "needkey", "key_env": "NOPE", "cost_tier": "high"}]}}
        r2, _, _ = rank(empty, "x", {}, False)
        if r2:
            failures.append("키 없는 단일 후보가 가용으로 잡힘")
        # 3) task_fit: cinematic 의도 → seedance(키 필요)는 제외됐으니 stock 중 best_for 매치 확인
        #    동의어: 'trailer' 의도가 seedance best_for 'cinematic'과 매치되는지(키 복구 후)
    finally:
        if saved is not None:
            os.environ["FAL_KEY"] = saved
        else:
            os.environ["FAL_KEY"] = "selftest-dummy"
    # 키 복구된 상태(dummy)에서:
    # 4) cinematic 의도 → fal-seedance(premium·best_for cinematic)가 1위여야
    ranked, _, _ = _rank({"intent": "cinematic trailer"})
    if not ranked or ranked[0]["id"] != "fal-seedance":
        failures.append("cinematic 의도인데 seedance가 1위가 아님: %s" % (ranked[0]["id"] if ranked else None))
    # 5) free-first: free 모드면 같은 b-roll 의도에서 free(stock/wan)가 fal보다 위로
    rf, _, _ = _rank({"intent": "b-roll"}, free_first=True)
    if rf and rf[0]["cost_tier"] != "free":
        failures.append("free-first인데 1위가 free가 아님: %s" % rf[0]["id"])
    # 6) 사용자 선호 override: prefer=fal-kling이면 강제 1위
    rp, _, forced = _rank({"intent": "cinematic", "prefer": "fal-kling"})
    if not forced or rp[0]["id"] != "fal-kling":
        failures.append("사용자 선호 override 실패")
    # 7) 무점수 채널: 출력에 0-100 정수 grade가 없어야(fit는 0~1 라우팅 적합도)
    with contextlib.redirect_stdout(sink):
        cmd_rank(cat, argparse.Namespace(capability="video_generation", intent="b-roll",
                 style="", prefer=None, locked=None, free_first=False, json=True))
    if re.search(r'"(score|grade|rating)"\s*:', sink.getvalue()):
        failures.append("출력에 금지된 score/grade/rating 키 존재")
    # 8) 결정론: 같은 입력 두 번 → 동일 랭킹 순서
    a = [r["id"] for r in _rank({"intent": "b-roll"})[0]]
    b = [r["id"] for r in _rank({"intent": "b-roll"})[0]]
    if a != b:
        failures.append("비결정 랭킹")

    # 9) 로컬 런타임 준비성 게이트(W0-4): probe 미선언=ready, bin/module 실측
    def rt(name, prov, want):
        ready, _ = runtime_ready(prov)
        if ready != want:
            failures.append("runtime_ready %s: ready=%s want=%s" % (name, ready, want))

    rt("no-probe", {"id": "x"}, True)                                   # probe 없으면 ready(레거시 보존)
    rt("bin-present", {"probe": {"bin": "sh"}}, True)                   # sh는 어디나 PATH에
    rt("bin-absent", {"probe": {"bin": "no_such_bin_xyz123"}}, False)
    rt("module-present", {"probe": {"module": "json"}}, True)           # stdlib 항상 import 가능
    rt("module-absent", {"probe": {"module": "no_such_mod_xyz123"}}, False)
    rt("any-bin-ok", {"probe": {"any_bin": ["no_x", "sh"]}}, True)
    rt("any-bin-no", {"probe": {"any_bin": ["no_x", "no_y"]}}, False)
    rt("path-absent", {"probe": {"path": "/no/such/path/xyz123"}}, False)
    rt("combined-fail", {"probe": {"bin": "sh", "module": "no_such_mod_xyz123"}}, False)

    # 10) 미준비 local provider는 available=False → 랭킹 제외·unavailable(runtime 사유)
    cat_rt = {"capabilities": {"scene-cut": [
        {"id": "scenecut_ready", "cost_tier": "free", "runtime": "local", "best_for": ["cuts"]},
        {"id": "scenecut_unready", "cost_tier": "free", "runtime": "local", "best_for": ["cuts"],
         "probe": {"bin": "no_such_bin_xyz123"}},
    ]}}
    rr, ru, _ = rank(cat_rt, "scene-cut", {"intent": "cuts"}, False)
    ids = {r["id"] for r in rr}
    if "scenecut_unready" in ids:
        failures.append("미준비 local provider가 가용으로 랭킹됨(W0-4 게이트 실패)")
    if "scenecut_ready" not in ids:
        failures.append("준비된 local provider가 빠짐")
    if not any(u["id"] == "scenecut_unready" and u.get("reason") == "runtime" for u in ru):
        failures.append("미준비 provider가 runtime 사유로 unavailable에 없음")

    # 11) 프로바이더 계약 적합성 린트(W1-3 verify)
    def vc(name, cat, want_err, want_warn=None):
        e, wn = verify_catalog(cat)
        if bool(e) != want_err:
            failures.append("verify %s: errors=%s want_err=%s (%s)" % (name, bool(e), want_err, e))
        if want_warn is not None and bool(wn) != want_warn:
            failures.append("verify %s: warnings=%s want_warn=%s (%s)" % (name, bool(wn), want_warn, wn))

    good_prov = {"id": "p1", "runtime": "api", "cost_tier": "free", "quality_tier": "good",
                 "key_env": "K", "best_for": ["x"], "supports": {}}
    vc("good", {"capabilities": {"c": [good_prov]}}, False, False)
    vc("no-caps", {"capabilities": {}}, True)
    vc("unknown-key", {"capabilities": {"c": [dict(good_prov, wat=1)]}}, True)
    vc("missing-runtime", {"capabilities": {"c": [{"id": "p", "cost_tier": "free"}]}}, True)
    vc("bad-runtime", {"capabilities": {"c": [{"id": "p", "runtime": "quantum", "cost_tier": "free"}]}}, True)
    vc("bad-cost", {"capabilities": {"c": [{"id": "p", "runtime": "api", "cost_tier": "cheap"}]}}, True)
    vc("bad-quality", {"capabilities": {"c": [dict(good_prov, quality_tier="meh")]}}, True)
    vc("bad-keyenv", {"capabilities": {"c": [dict(good_prov, key_env="")]}}, True)
    vc("bad-probe", {"capabilities": {"c": [{"id": "p", "runtime": "local", "cost_tier": "free",
                                             "probe": {"bin": ""}}]}}, True)
    vc("probe-unknown", {"capabilities": {"c": [{"id": "p", "runtime": "local", "cost_tier": "free",
                                                 "probe": {"gpu": "cuda"}}]}}, True)
    # 무키 local + probe 없음 → 에러 아님, 경고
    vc("local-no-probe-warn", {"capabilities": {"c": [{"id": "p", "runtime": "local",
                                                       "cost_tier": "free"}]}}, False, True)

    os.environ.pop("FAL_KEY", None)  # 청소

    print(json.dumps({"self_test": "ok" if not failures else "fail",
                      "failures": failures}, ensure_ascii=False, indent=2))
    return 0 if not failures else 1


def main():
    ap = argparse.ArgumentParser(description="채점식 provider 선택 엔진 (도메인-무관·결정론)")
    ap.add_argument("--self-test", action="store_true")
    sub = ap.add_subparsers(dest="cmd")

    r = sub.add_parser("rank", help="capability 가용 provider 랭킹 (0=결정 1=가용없음)")
    r.add_argument("--catalog", required=True)
    r.add_argument("--capability", required=True)
    r.add_argument("--intent", default="")
    r.add_argument("--style", default="", help="콤마 구분 스타일 키워드")
    r.add_argument("--prefer", default=None, help="사용자 선호 provider id(가용 시 1위 강제)")
    r.add_argument("--locked", default=None, help="이미 잠긴 provider id(연속성 가점)")
    r.add_argument("--free-first", action="store_true", help="무료(로컬·스톡) 우선")
    r.add_argument("--json", action="store_true")

    m = sub.add_parser("menu", help="capability별 N-of-M 가용 메뉴(정직한 능력봉투)")
    m.add_argument("--catalog", required=True)
    m.add_argument("--json", action="store_true")

    ve = sub.add_parser("verify", help="프로바이더 계약 적합성 린트 (0=준수 1=위반 2=입출력)")
    ve.add_argument("--catalog", required=True)
    ve.add_argument("--json", action="store_true")

    args = ap.parse_args()
    if args.self_test:
        return self_test()
    if args.cmd in ("rank", "menu", "verify"):
        try:
            catalog = load_catalog(args.catalog)
        except (OSError, json.JSONDecodeError) as e:
            return fail(2, "카탈로그 로드 실패: %s" % e)
        if args.cmd == "rank":
            return cmd_rank(catalog, args)
        if args.cmd == "verify":
            return cmd_verify(catalog, args.json)
        return cmd_menu(catalog, args.json)
    ap.print_help()
    return 2


if __name__ == "__main__":
    sys.exit(main())
