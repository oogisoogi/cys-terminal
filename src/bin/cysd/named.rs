//! 이름 있는 보고자(named reporter) — surface 없는 Claude의 ctx 관측.
//!
//! 오너 요청 2026-08-07: 「master·CSO 의 CTX 도 사이드바 페인 CTX 절에 표시. 라벨은 번호 대신
//! master·cso」.
//!
//! ★왜 새 저장소가 필요한가(실측 근거 2026-08-07): master·CSO는 cys surface가 아니라 cmux 페인의
//! Claude다. 이들의 statusline은 `cys usage-report-stdin`을 타지만, 그 함수는
//! `if let Ok(sid) = target_surface(..)` 안에서만 push한다(cys.rs). cmux 페인에는 `--surface`도
//! `CYS_SURFACE_ID`도 없으므로 **push 자체가 실행되지 않는다** — `env -u CYS_SURFACE_ID`로 재현해
//! 확인했다(사람용 줄만 나오고 RPC는 0건). 데몬의 `usage.report` 처리도 전부 surface 조회 블록
//! 안이라 sid 없이는 들어올 문이 없다.
//!
//! ⚠계정 저장소가 차 있는 것은 이 경로의 증거가 **아니다**: accounts.rs `seed_known`이 부팅 때
//! 홈의 `.claude*`·`.cys/claude*` 디렉터리를 직접 스캔해 계정 신원을 심기 때문이다.
//! 문 앞까지 온 것은 **계정 이름**이지 ctx가 아니었다. 그래서 전송로부터 새로 낸다.

use serde_json::{json, Value};
use std::collections::BTreeMap;

/// 보고자 1건의 최신 관측. surface의 ObservedUsage와 같은 축을 갖되 rate는 없다 —
/// rate(5h·7d)는 계정 저장소가 이미 계정 단위로 들고 있어 여기서 또 세면 경계가 섞인다.
#[derive(Clone, Debug)]
pub struct NamedReport {
    pub ctx_pct: Option<f64>,
    pub ctx_tokens: Option<u64>,
    pub ctx_window: Option<u64>,
    pub source: String,
    pub updated_at: f64,
}

#[derive(Default)]
pub struct NamedState {
    /// 이름 → 최신 관측. BTreeMap이라 노출 순서가 이름순으로 결정론이다.
    pub reports: BTreeMap<String, NamedReport>,
}

/// 기본 매핑 — 경로 접두 → 보고자 이름.
///
/// ★오너가 지정한 두 개가 기본값이다. env `CYS_NAMED_REPORTERS`로 재정의할 수 있게 둔 이유는
/// 이 경로가 이 기계의 사정이기 때문이다 — 코드에 박으면 다른 기계에서 조용히 아무도 안 잡힌다.
const DEFAULT_MAP: [(&str, &str); 2] = [
    ("/Users/oogisoogi/axdev/cso", "cso"),
    ("/Users/oogisoogi/axdev", "master"),
];

/// env 형식: `<경로>=<이름>` 을 `:` 로 이어 붙인다.
/// 예) `CYS_NAMED_REPORTERS=/Users/o/axdev=master:/Users/o/axdev/cso=cso`
fn mapping() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = match std::env::var("CYS_NAMED_REPORTERS") {
        Ok(s) if !s.trim().is_empty() => s
            .split(':')
            .filter_map(|pair| {
                let (p, n) = pair.split_once('=')?;
                let (p, n) = (p.trim(), n.trim());
                if p.is_empty() || n.is_empty() {
                    return None;
                }
                Some((p.trim_end_matches('/').to_string(), n.to_string()))
            })
            .collect(),
        _ => DEFAULT_MAP
            .iter()
            .map(|(p, n)| (p.to_string(), n.to_string()))
            .collect(),
    };
    // ★긴 경로가 먼저 오게 정렬한다 — 최장 접두 일치를 위해서다.
    //   이게 없으면 `/…/axdev` 가 `/…/axdev/cso` 를 먼저 먹어 CSO 가 영원히 master로 보고된다
    //   (부모가 자식을 삼키는 고전적 접두 버그).
    out.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    out
}

/// cwd → 보고자 이름. 판별 불가면 None.
///
/// ★None일 때 이름을 지어내지 않는다(오너 지시). 모르는 보고자를 「unknown」 같은 라벨로 띄우면
/// 사용자는 그것을 실재하는 노드로 읽는다 — 관측되지 않은 것을 화면에 만들어 내는 셈이다.
pub fn resolve_name(cwd: &str) -> Option<String> {
    if cwd.is_empty() {
        return None;
    }
    let cwd = cwd.trim_end_matches('/');
    for (prefix, name) in mapping() {
        // 정확히 그 경로이거나 그 아래여야 한다. 단순 starts_with면 `/axdev-old`가
        // `/axdev`에 걸린다(형제 디렉터리 오인) — 경계 문자까지 봐야 한다.
        if cwd == prefix || cwd.starts_with(&format!("{prefix}/")) {
            return Some(name);
        }
    }
    None
}

/// 관측 적재. 이름이 판별된 경우에만 호출된다.
pub fn note(state: &mut NamedState, name: &str, r: NamedReport) {
    state.reports.insert(name.to_string(), r);
}

/// UI 노출용 JSON. surface 행과 같은 축(신선도·출처)을 실어 보내 같은 규율로 그릴 수 있게 한다.
pub fn to_json(state: &NamedState) -> Value {
    Value::Array(
        state
            .reports
            .iter()
            .map(|(name, r)| {
                json!({
                    "name": name,
                    "ctx_pct": r.ctx_pct,
                    "ctx_tokens": r.ctx_tokens,
                    "ctx_window": r.ctx_window,
                    "source": r.source,
                    "updated_at": r.updated_at,
                })
            })
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★env를 읽는 경로의 테스트는 **한 함수 안에서 순차로** 돌린다.
    ///
    /// 초판은 이것을 세 개의 #[test]로 나눴다가 간헐 적색을 만들었다 — 환경변수는 프로세스
    /// 전역인데 cargo는 테스트를 스레드로 병렬 실행하므로, 한 테스트가 set_var 한 값을
    /// 다른 테스트가 보고 넘어진다. 스케줄에 따라 통과·실패가 갈려 「가끔 빨간」 테스트가 됐다.
    /// ★가끔 빨간 테스트는 없느니만 못하다 — 사람이 빨간불을 무시하는 법을 배우게 만든다.
    #[test]
    fn resolve_name_mapping_rules() {
        std::env::remove_var("CYS_NAMED_REPORTERS");
        // ① 기본 매핑 — 오너가 지정한 두 경로.
        assert_eq!(resolve_name("/Users/oogisoogi/axdev").as_deref(), Some("master"));
        // ★최장 접두 — cso가 master에 먹히면 안 된다(정렬이 없으면 여기서 master가 나온다).
        assert_eq!(resolve_name("/Users/oogisoogi/axdev/cso").as_deref(), Some("cso"));
        assert_eq!(resolve_name("/Users/oogisoogi/axdev/cso/sub").as_deref(), Some("cso"));
        // 하위 경로도 그 역할로 본다(세션이 하위 폴더에서 열려 있을 수 있다).
        assert_eq!(resolve_name("/Users/oogisoogi/axdev/eduscan").as_deref(), Some("master"));

        // ② 판별 불가 — 라벨을 짓지 않는다.
        assert_eq!(resolve_name("/Users/oogisoogi/cys-terminal-src"), None);
        assert_eq!(resolve_name(""), None);
        // 형제 디렉터리를 접두로 오인하지 않는다(`/axdev-old`가 `/axdev`에 걸리면 안 된다).
        assert_eq!(resolve_name("/Users/oogisoogi/axdev-old"), None);

        // ③ env 재정의 — 치환이지 병합이 아니다.
        std::env::set_var("CYS_NAMED_REPORTERS", "/tmp/a=alpha:/tmp/a/b=beta");
        assert_eq!(resolve_name("/tmp/a").as_deref(), Some("alpha"));
        assert_eq!(resolve_name("/tmp/a/b").as_deref(), Some("beta"));
        assert_eq!(resolve_name("/Users/oogisoogi/axdev"), None, "재정의하면 기본값은 안 걸린다");
        std::env::remove_var("CYS_NAMED_REPORTERS");
    }

    #[test]
    fn note_and_expose_roundtrip() {
        let mut st = NamedState::default();
        note(
            &mut st,
            "master",
            NamedReport {
                ctx_pct: Some(11.0),
                ctx_tokens: Some(22_000),
                ctx_window: Some(200_000),
                source: "statusline".into(),
                updated_at: 1000.0,
            },
        );
        let v = to_json(&st);
        assert_eq!(v[0]["name"], "master");
        assert_eq!(v[0]["ctx_pct"], 11.0);
        assert_eq!(v[0]["source"], "statusline");
        // 같은 이름의 새 관측은 덮어쓴다(최신만 남는다 — 행이 늘어나지 않는다).
        note(
            &mut st,
            "master",
            NamedReport {
                ctx_pct: Some(12.0),
                ctx_tokens: None,
                ctx_window: None,
                source: "statusline".into(),
                updated_at: 2000.0,
            },
        );
        let v = to_json(&st);
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["ctx_pct"], 12.0);
    }
}
