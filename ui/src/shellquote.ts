// 드롭·붙여넣기 경로를 PTY로 타이핑할 때 셸이 경로를 '단일 인자'로 안전하게 받도록 인용한다 — 순수 함수.
//
// main.ts의 드롭/붙여넣기 핸들러는 이 함수에 배선만 하고, navigator 플랫폼 판별은 호출측이 넘긴다(테스트 격리).
// 순수 함수라 공백·따옴표·한글 경로를 결정론으로 회귀 테스트할 수 있다(shellquote.test.ts).

// POSIX 셸: 작은따옴표로 감싸고 내부 '만 '\'' 로 끊어 이스케이프한다.
// 작은따옴표 안에서는 $·"·\·공백·한글이 전부 리터럴이라 안전. 빈 문자열도 ''로 유효.
export function shellQuotePosix(p: string): string {
  return "'" + p.replace(/'/g, "'\\''") + "'";
}

// Windows(cmd/PowerShell): 큰따옴표로 감싸고 내부 "는 "" 로 이스케이프(양 셸 공통 관용).
// 공백·한글 경로는 큰따옴표만으로 단일 인자 보장.
export function shellQuoteWindows(p: string): string {
  return '"' + p.replace(/"/g, '""') + '"';
}

// 플랫폼(isWindows)에 맞는 인용을 고른다.
export function shellQuote(p: string, isWindows: boolean): string {
  return isWindows ? shellQuoteWindows(p) : shellQuotePosix(p);
}

// 여러 경로를 각각 인용해 공백으로 연결(드롭 다중 파일).
export function shellQuoteJoin(paths: string[], isWindows: boolean): string {
  return paths.map((p) => shellQuote(p, isWindows)).join(" ");
}
