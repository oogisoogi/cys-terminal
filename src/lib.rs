//! cys (CYSJavis Terminal) — shared protocol types, socket path resolution, and key mapping.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

pub mod action_catalog;
pub mod directive_compose;
pub mod edit_kinds;
pub mod pack;
pub mod overrides;
pub mod wire;
#[cfg(target_os = "macos")]
pub mod launchd;

pub const ENV_SOCKET: &str = "CYS_SOCKET";
pub const ENV_SURFACE_ID: &str = "CYS_SURFACE_ID";
pub const ENV_SURFACE_REF: &str = "CYS_SURFACE_REF";
pub const ENV_ROLE: &str = "CYS_ROLE";

/// 이행기 호환: CYS_* 우선 → 구 JAVIS_* → 구 AITERM_* 순 폴백.
pub fn env_compat(primary: &str) -> Option<String> {
    let javis = primary.replacen("CYS_", "JAVIS_", 1);
    let aiterm = primary.replacen("CYS_", "AITERM_", 1);
    [primary, javis.as_str(), aiterm.as_str()]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

/// Wire protocol: one JSON object per line (NDJSON), request/response with id echo.
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

pub fn ok_response(id: &Value, result: Value) -> Value {
    serde_json::json!({"id": id, "ok": true, "result": result})
}

pub fn err_response(id: &Value, code: &str, message: &str) -> Value {
    serde_json::json!({"id": id, "ok": false, "error": {"code": code, "message": message}})
}

/// Default socket path: ~/.local/state/cys/cys.sock (unix),
/// \\.\pipe\cys (windows). Overridable via CYS_SOCKET (legacy JAVIS_/AITERM_ honored).
pub fn socket_path() -> PathBuf {
    if let Some(p) = env_compat(ENV_SOCKET) {
        return PathBuf::from(p);
    }
    #[cfg(windows)]
    {
        PathBuf::from(r"\\.\pipe\cys")
    }
    #[cfg(not(windows))]
    {
        let base = dirs::state_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let dir = if base.ends_with(".local/state") || base.to_string_lossy().contains("state") {
            base.join("cys")
        } else {
            base.join(".local/state/cys")
        };
        dir.join("cys.sock")
    }
}

/// Parse a surface reference: "surface:31", "31", or 31 → 31.
pub fn parse_surface_ref(s: &str) -> Option<u64> {
    let t = s.trim();
    let t = t.strip_prefix("surface:").unwrap_or(t);
    t.parse::<u64>().ok()
}

pub fn surface_ref(id: u64) -> String {
    format!("surface:{id}")
}

/// Map a named key name to the byte sequence
/// written to the PTY. Supports C- (ctrl), M- (alt/meta) prefixes.
pub fn key_to_bytes(key: &str) -> Option<Vec<u8>> {
    // Modifier prefixes
    if let Some(rest) = key.strip_prefix("C-") {
        // 단일 문자일 때만 ctrl 비트 변환 — "C-Space"의 'S'가 0x13(XOFF, 출력 동결)으로
        // 잘못 변환되어 Space 분기가 사문화되는 것을 차단
        if rest.chars().count() == 1 {
            let c = rest.chars().next()?;
            let lower = c.to_ascii_lowercase();
            if lower.is_ascii_lowercase() {
                return Some(vec![(lower as u8) & 0x1f]);
            }
        }
        return match rest {
            "Space" | "space" => Some(vec![0x00]),
            _ => None,
        };
    }
    if let Some(rest) = key.strip_prefix("M-") {
        let mut b = vec![0x1b];
        b.extend_from_slice(rest.as_bytes());
        return Some(b);
    }
    let seq: &[u8] = match key {
        "Return" | "Enter" => b"\r",
        "Tab" => b"\t",
        "BTab" | "BackTab" => b"\x1b[Z",
        "Space" => b" ",
        "Escape" | "Esc" => b"\x1b",
        "Backspace" => b"\x7f",
        "Delete" | "DC" => b"\x1b[3~",
        "Up" => b"\x1b[A",
        "Down" => b"\x1b[B",
        "Right" => b"\x1b[C",
        "Left" => b"\x1b[D",
        "Home" => b"\x1b[H",
        "End" => b"\x1b[F",
        "PageUp" | "PPage" => b"\x1b[5~",
        "PageDown" | "NPage" => b"\x1b[6~",
        "F1" => b"\x1bOP",
        "F2" => b"\x1bOQ",
        "F3" => b"\x1bOR",
        "F4" => b"\x1bOS",
        "F5" => b"\x1b[15~",
        "F6" => b"\x1b[17~",
        "F7" => b"\x1b[18~",
        "F8" => b"\x1b[19~",
        "F9" => b"\x1b[20~",
        "F10" => b"\x1b[21~",
        "F11" => b"\x1b[23~",
        "F12" => b"\x1b[24~",
        _ => {
            // Single literal character passes through
            if key.chars().count() == 1 {
                return Some(key.as_bytes().to_vec());
            }
            return None;
        }
    };
    Some(seq.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_refs() {
        assert_eq!(parse_surface_ref("surface:31"), Some(31));
        assert_eq!(parse_surface_ref("31"), Some(31));
        assert_eq!(parse_surface_ref("x"), None);
    }

    #[test]
    fn keys() {
        assert_eq!(key_to_bytes("Return"), Some(b"\r".to_vec()));
        assert_eq!(key_to_bytes("C-c"), Some(vec![0x03]));
        assert_eq!(key_to_bytes("Up"), Some(b"\x1b[A".to_vec()));
    }

    #[test]
    fn surface_ref_roundtrip_and_edges() {
        // 왕복: id → surface_ref → parse_surface_ref → id
        for id in [0u64, 1, 31, 65535, u64::MAX] {
            assert_eq!(parse_surface_ref(&surface_ref(id)), Some(id));
        }
        // 공백 trim
        assert_eq!(parse_surface_ref("  42  "), Some(42));
        assert_eq!(parse_surface_ref("\tsurface:7\n"), Some(7));
        // prefix는 1회만 제거 — 이중 prefix는 parse 실패
        assert_eq!(parse_surface_ref("surface:surface:31"), None);
        // 음수·비숫자·빈 문자열
        assert_eq!(parse_surface_ref("-5"), None);
        assert_eq!(parse_surface_ref(""), None);
        assert_eq!(parse_surface_ref("surface:"), None);
        assert_eq!(parse_surface_ref("3.5"), None);
        // u64 초과는 None (오버플로 시 silent wrap 금지)
        assert_eq!(parse_surface_ref("18446744073709551616"), None);
    }

    #[test]
    fn key_ctrl_modifier() {
        // C-c == C-C (대소문자 무관, ctrl 비트 0x1f 마스크)
        assert_eq!(key_to_bytes("C-c"), Some(vec![0x03]));
        assert_eq!(key_to_bytes("C-C"), Some(vec![0x03]));
        assert_eq!(key_to_bytes("C-a"), Some(vec![0x01]));
        assert_eq!(key_to_bytes("C-z"), Some(vec![0x1a]));
        // C-Space → NUL (0x00), 'S'가 0x13(XOFF)으로 오변환되지 않음
        assert_eq!(key_to_bytes("C-Space"), Some(vec![0x00]));
        assert_eq!(key_to_bytes("C-space"), Some(vec![0x00]));
        // ctrl + 비-알파벳 단일문자는 매핑 없음
        assert_eq!(key_to_bytes("C-1"), None);
        assert_eq!(key_to_bytes("C-["), None);
        // 다중문자 C- (Space 외)는 ctrl 비트 변환 금지 → None
        assert_eq!(key_to_bytes("C-Foo"), None);
        // C- + 비-ASCII 단일문자(멀티바이트)는 ctrl 매핑 없음 → None
        // (count==1이라 단일문자 분기에 들지만 is_ascii_lowercase=false라 fall-through)
        assert_eq!(key_to_bytes("C-가"), None);
        // C- 단독(빈 rest)은 단일문자도 Space도 아님 → None
        assert_eq!(key_to_bytes("C-"), None);
    }

    #[test]
    fn key_meta_modifier() {
        // M-x → ESC + 'x'
        assert_eq!(key_to_bytes("M-x"), Some(vec![0x1b, b'x']));
        // M-<여러글자>도 ESC 접두 후 그대로 (Alt 시퀀스)
        assert_eq!(
            key_to_bytes("M-Foo"),
            Some([&[0x1b][..], b"Foo"].concat())
        );
        // M- 단독 (빈 rest) → ESC 단독
        assert_eq!(key_to_bytes("M-"), Some(vec![0x1b]));
    }

    #[test]
    fn key_named_and_literal() {
        assert_eq!(key_to_bytes("Enter"), Some(b"\r".to_vec()));
        assert_eq!(key_to_bytes("Tab"), Some(b"\t".to_vec()));
        assert_eq!(key_to_bytes("Escape"), Some(b"\x1b".to_vec()));
        assert_eq!(key_to_bytes("Backspace"), Some(b"\x7f".to_vec()));
        assert_eq!(key_to_bytes("F5"), Some(b"\x1b[15~".to_vec()));
        // 단일 리터럴 문자는 그대로 통과 (멀티바이트 포함)
        assert_eq!(key_to_bytes("a"), Some(b"a".to_vec()));
        assert_eq!(key_to_bytes("가"), Some("가".as_bytes().to_vec()));
        // 알 수 없는 다중문자 키 이름 → None
        assert_eq!(key_to_bytes("Nonsense"), None);
        assert_eq!(key_to_bytes(""), None);
    }

    #[test]
    fn key_function_keys_use_correct_protocol() {
        // F1-F4는 SS3(\x1bO_), F5+는 CSI(\x1b[_~) — 두 인코딩이 갈리는 경계 박제.
        assert_eq!(key_to_bytes("F1"), Some(b"\x1bOP".to_vec()));
        assert_eq!(key_to_bytes("F4"), Some(b"\x1bOS".to_vec()));
        assert_eq!(key_to_bytes("F5"), Some(b"\x1b[15~".to_vec()));
        assert_eq!(key_to_bytes("F12"), Some(b"\x1b[24~".to_vec()));
        // F5와 F6 사이에 16이 건너뛰는 VT 표준(역사적 결번) 보존
        assert_eq!(key_to_bytes("F6"), Some(b"\x1b[17~".to_vec()));
        // 대소문자 민감 — 'f1'은 명명키 아님, 단일문자도 아님(2글자) → None
        assert_eq!(key_to_bytes("f1"), None);
    }

    #[test]
    fn key_navigation_and_aliases() {
        // 화살표(CSI 종결바이트 A-D)
        assert_eq!(key_to_bytes("Up"), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_to_bytes("Down"), Some(b"\x1b[B".to_vec()));
        assert_eq!(key_to_bytes("Right"), Some(b"\x1b[C".to_vec()));
        assert_eq!(key_to_bytes("Left"), Some(b"\x1b[D".to_vec()));
        // 별칭 동치 (Return=Enter 등 호환 어휘)
        assert_eq!(key_to_bytes("Return"), key_to_bytes("Enter"));
        assert_eq!(key_to_bytes("Esc"), key_to_bytes("Escape"));
        assert_eq!(key_to_bytes("BTab"), key_to_bytes("BackTab"));
        assert_eq!(key_to_bytes("Delete"), key_to_bytes("DC"));
        assert_eq!(key_to_bytes("PageUp"), key_to_bytes("PPage"));
        assert_eq!(key_to_bytes("PageDown"), key_to_bytes("NPage"));
        // BTab은 CSI Z (shift-tab)
        assert_eq!(key_to_bytes("BTab"), Some(b"\x1b[Z".to_vec()));
    }

    #[test]
    fn key_meta_with_named_key_is_literal_not_translated() {
        // ★불변식 박제: M- 접두는 rest를 명명키로 재해석하지 않고 '리터럴 바이트'로 붙인다.
        // 즉 M-Enter는 ESC+CR(\x1b\r)이 아니라 ESC + "Enter"(\x1b + 5글자)다.
        // (이 동작에 의존하는 호출부가 있으면 회귀 시 여기서 드러난다)
        assert_eq!(key_to_bytes("M-Enter"), Some([&[0x1b][..], b"Enter"].concat()));
        assert_ne!(key_to_bytes("M-Enter"), Some(vec![0x1b, b'\r']));
        // M-멀티바이트도 UTF-8 바이트 그대로 ESC 뒤에 (Alt+한글)
        assert_eq!(
            key_to_bytes("M-가"),
            Some([&[0x1b][..], "가".as_bytes()].concat())
        );
    }

    #[test]
    fn env_compat_fallback_priority() {
        // 고유 키로 격리 (다른 테스트·환경과 충돌 방지)
        let p = "CYS_ZZUNIQUETEST";
        let j = "JAVIS_ZZUNIQUETEST";
        let a = "AITERM_ZZUNIQUETEST";
        for k in [p, j, a] {
            std::env::remove_var(k);
        }
        // 셋 다 없으면 None
        assert_eq!(env_compat(p), None);
        // AITERM_만 있으면 폴백
        std::env::set_var(a, "aiterm_val");
        assert_eq!(env_compat(p), Some("aiterm_val".to_string()));
        // JAVIS_가 AITERM_보다 우선
        std::env::set_var(j, "javis_val");
        assert_eq!(env_compat(p), Some("javis_val".to_string()));
        // CYS_(primary)가 최우선
        std::env::set_var(p, "cys_val");
        assert_eq!(env_compat(p), Some("cys_val".to_string()));
        // 빈 문자열은 미설정으로 간주 → 다음 폴백
        std::env::set_var(p, "");
        assert_eq!(env_compat(p), Some("javis_val".to_string()));
        for k in [p, j, a] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn env_compat_only_first_cys_token_is_rewritten() {
        // replacen(..,1)이 'CYS_'를 첫 1회만 치환 — primary에 CYS_가 없으면
        // 세 후보 키가 모두 primary와 동일(폴백 무의미)임을 박제.
        let only = "CYS_ZZONLYPRIMARY";
        let javis = "JAVIS_ZZONLYPRIMARY";
        std::env::remove_var(only);
        std::env::remove_var(javis);
        // primary에 CYS_가 없는 키: 폴백 키가 자기 자신과 같아져 primary만 본다
        let nocys = "PLAINKEY_ZZ";
        std::env::remove_var(nocys);
        assert_eq!(env_compat(nocys), None);
        std::env::set_var(nocys, "plain");
        assert_eq!(env_compat(nocys), Some("plain".to_string()));
        std::env::remove_var(nocys);
        // 첫 CYS_만 치환 — 'CYS_'가 값 중간에 또 나와도 1회만
        std::env::set_var(javis, "via_javis");
        assert_eq!(env_compat(only), Some("via_javis".to_string()));
        std::env::remove_var(javis);
    }
}
