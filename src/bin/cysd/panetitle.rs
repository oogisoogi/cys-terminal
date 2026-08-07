//! 페인 제목의 모델 조각 — 「번호 · 모델 · 특성」의 가운데 한 칸을 데몬이 관리한다.
//!
//! 오너 요청 2026-08-07: 「푸터에 모델만 있으니 어색하다. 모델은 제목에 넣자.
//! 페인번호 + 모델 + 제목키워드 형식」.
//!
//! ★왜 데몬인가(정적 1회 기록이 아닌 이유): 모델은 세션 중에 `/model`로 바뀐다. 기동 시 한 번
//! 적어 넣으면 그 순간부터 제목이 거짓말을 한다 — 화면에는 Opus라고 적혀 있는데 실제로는
//! Sonnet이 도는 상태가 조용히 이어진다. 데몬은 statusline의 usage.report를 매 턴 받으므로,
//! 그 model 값으로 조각을 갱신하면 제목이 **관측을 따라간다**.
//!
//! ★소유권 경계: 번호와 특성은 pack의 javis_panetitle.py가 정한다(surface.rename). 이 모듈은
//! **가운데 한 칸만** 건드리고 그 두 조각은 원문 그대로 옮긴다. 두 주인이 같은 문자열을 쓰지만
//! 서로 다른 칸을 쓰므로 충돌하지 않는다 — panetitle.py는 「번호로 시작하면 무접촉」이라
//! 우리가 넣은 모델 조각을 지우지 않는다(그 스크립트의 멱등 규칙이 그대로 우리를 보호한다).

/// 제목에 넣는 모델 조각의 닫힌 어휘.
///
/// ★열린 어휘(display_name을 그대로 넣기)를 쓰지 않는 이유: 조각을 **다시 알아볼 수 있어야**
/// 다음 갱신 때 교체가 되는데, 아무 문자열이나 허용하면 「특성」 칸과 구별할 수 없다.
/// 그러면 모델이 바뀔 때마다 조각이 하나씩 늘어난다(372 · Opus · Sonnet · cys-terminal-src).
/// 닫힌 어휘는 그 재귀 오염을 구조적으로 막는다.
const MODEL_FAMILIES: [&str; 4] = ["Opus", "Sonnet", "Haiku", "Fable"];

/// 구분자 — javis_panetitle.py가 쓰는 것과 같아야 한다(제목은 두 주인이 공유하는 문자열이다).
const SEP: &str = " · ";

/// statusline의 `model.display_name` → 제목 조각.
///
/// 아는 계열이 없으면 None이다. ★모르는 모델에 이름을 붙여 주지 않는다 —
/// 없는 값을 지어내는 것은 관측이 아니라 창작이다(오너 지시: 모델 미관측이면 조각 생략).
pub fn model_segment(display_name: &str) -> Option<&'static str> {
    let low = display_name.to_lowercase();
    MODEL_FAMILIES
        .iter()
        .copied()
        .find(|f| low.contains(&f.to_lowercase()))
}

/// 조각 하나가 모델 칸인가 — 닫힌 어휘와 **완전 일치**로만 판정한다.
///
/// ★부분 일치를 쓰면 「opus-notes」라는 폴더명이 모델 조각으로 오인돼 특성이 지워진다.
fn is_model_segment(seg: &str) -> bool {
    MODEL_FAMILIES.iter().any(|f| f.eq_ignore_ascii_case(seg))
}

/// 제목의 모델 조각을 관측된 모델로 맞춘다.
///
/// 반환 None = **바꿀 것이 없다**(호출자는 rename을 건너뛴다). 멱등이 이 함수의 계약이다 —
/// 매 턴 statusline이 들어오므로, 같은 값에 매번 rename을 쏘면 초당 몇 번씩 제목을 다시 쓰게 된다.
///
/// 무접촉 조건(하나라도 걸리면 None):
///  · 제목이 「숫자 ·」로 시작하지 않는다 ⇒ 번호 규칙 밖이다. 사용자가 지은 이름·자동 제목
///    (`surface 12`)·빈 제목이 여기 해당한다. ★남의 이름을 우리 규칙으로 덮지 않는다.
///  · 이미 원하는 모양이다.
pub fn retitle_with_model(title: &str, model: Option<&str>) -> Option<String> {
    let parts: Vec<&str> = title.split(SEP).collect();
    let num = parts.first()?;
    // 번호 규칙 안인지 — 첫 조각이 숫자만이어야 한다. 빈 제목·자동 제목은 여기서 걸러진다.
    if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    // 기존 모델 조각(있으면) 제거 — 두 번째 칸만 후보다. 뒤쪽 칸은 특성의 일부일 수 있다.
    let mut rest: Vec<&str> = parts[1..].to_vec();
    if rest.first().is_some_and(|s| is_model_segment(s)) {
        rest.remove(0);
    }
    let desired = model.and_then(model_segment);
    let mut out: Vec<&str> = vec![num];
    if let Some(m) = desired {
        out.push(m);
    }
    out.extend(rest);
    let next = out.join(SEP);
    if next == title {
        return None; // 멱등 — 같은 값에 rename을 쏘지 않는다
    }
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_model_between_number_and_keyword() {
        assert_eq!(
            retitle_with_model("372 · cys-terminal-src", Some("Opus 4.1")).as_deref(),
            Some("372 · Opus · cys-terminal-src")
        );
    }

    #[test]
    fn idempotent_when_already_correct() {
        // ★매 턴 statusline이 들어온다 — 같은 값에 None을 돌려주지 않으면 제목을 계속 다시 쓴다.
        assert_eq!(retitle_with_model("372 · Opus · cys-terminal-src", Some("Opus 4.1")), None);
    }

    #[test]
    fn follows_model_switch() {
        // /model 전환 추종 — 조각이 늘어나지 않고 교체된다.
        assert_eq!(
            retitle_with_model("372 · Opus · cys-terminal-src", Some("Claude Sonnet 5")).as_deref(),
            Some("372 · Sonnet · cys-terminal-src")
        );
        // 두 번 바꿔도 조각은 계속 하나다(재귀 오염 없음).
        assert_eq!(
            retitle_with_model("372 · Sonnet · cys-terminal-src", Some("Fable 5")).as_deref(),
            Some("372 · Fable · cys-terminal-src")
        );
    }

    #[test]
    fn unknown_model_adds_nothing() {
        // 지어내기 금지 — 모르는 이름은 조각을 만들지 않는다.
        assert_eq!(retitle_with_model("372 · cys-terminal-src", Some("gpt-5.6-sol")), None);
        assert_eq!(model_segment("gpt-5.6-sol"), None);
    }

    #[test]
    fn no_model_observed_leaves_title_alone() {
        // 셸 페인 등 모델 미관측 — 조각 생략.
        assert_eq!(retitle_with_model("371 · scripts", None), None);
    }

    #[test]
    fn no_model_observed_strips_stale_segment() {
        // 관측이 끊겼는데 옛 조각이 남아 있으면 지운다 — 없는 것을 있다고 적어 두지 않는다.
        assert_eq!(
            retitle_with_model("371 · Opus · scripts", None).as_deref(),
            Some("371 · scripts")
        );
    }

    #[test]
    fn leaves_user_named_titles_untouched() {
        // 번호 규칙 밖 = 남의 이름. 우리 규칙으로 덮지 않는다.
        assert_eq!(retitle_with_model("내 작업창", Some("Opus 4.1")), None);
        assert_eq!(retitle_with_model("surface 12", Some("Opus 4.1")), None);
        assert_eq!(retitle_with_model("", Some("Opus 4.1")), None);
    }

    #[test]
    fn keyword_containing_family_word_is_not_eaten() {
        // ★부분 일치였다면 여기서 특성이 지워졌다 — 완전 일치 판정의 존재 이유.
        assert_eq!(
            retitle_with_model("372 · opus-notes", Some("Opus 4.1")).as_deref(),
            Some("372 · Opus · opus-notes")
        );
    }

    #[test]
    fn number_only_title_gets_model() {
        assert_eq!(
            retitle_with_model("372", Some("Opus 4.1")).as_deref(),
            Some("372 · Opus")
        );
    }
}
