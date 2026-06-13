# THIRD_PARTY — 외부 유래 스킬 출처 고지

아래 14종 스킬은 [NomaDamas/k-skill](https://github.com/NomaDamas/k-skill) (MIT License)에서
vendoring했다. 업스트림 커밋 핀: `66f12cb43d833e4b9aa4593d430bd5524fff9d58` (2026-06-12).
채택 기준·감사 기록은 cysjavis 선별 감사(2026-06-12, 96종 중 14종 채택 — 실작동·보편성·
오류 절감력 기준) 참조. 갱신 시 업스트림 diff 후 동일 기준으로 재감사한다.

korean-humanizer · korean-spell-check · korean-character-count · naver-blog-research ·
kosis-stats · hwp · rhwp-edit · joseon-sillok-search · geeknews-search · k-dart ·
korean-patent-search · korean-stock-search · daishin-report-search · library-book-search

주의: kosis-stats·korean-stock-search·library-book-search 기본 경로는 운영자 프록시
(`k-skill-proxy.nomadamas.org`) 경유다 — `KSKILL_PROXY_BASE_URL`로 자가 호스팅 전환 가능,
kosis는 BYOK 직접 경로 지원. (korean-law-search·naver-news-search는 오너 결정으로 미채택 —
법령은 전용 MCP로 대체 예정.)

## MIT License (NomaDamas/k-skill)

MIT License

Copyright (c) 2026

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## obra/superpowers (MIT License, Copyright (c) 2025 Jesse Vincent)

아래 9종은 [obra/superpowers](https://github.com/obra/superpowers)에서 vendoring했다.
업스트림 커밋 핀: `6fd4507659784c351abbd2bc264c7162cfd386dc` (2026-05-29).
채택 기준: 2026-06-12 감사 — A급 5(pack 공백: systematic-debugging·test-driven-development·
subagent-driven-development·dispatching-parallel-agents·verification-before-completion) +
B급 4(보완: brainstorming·receiving-code-review·writing-plans·using-git-worktrees).
패치: `superpowers:` 플러그인 네임스페이스 접두 제거. 저작 부산물(CREATION-LOG·
test-pressure-*·test-academic*)은 제외. hook·부트스트랩 메타 스킬(using-superpowers)은
거버넌스 점유형이라 불채택 — 우리 디렉티브 주입 체계가 대체.

## mattpocock/skills (MIT License, Copyright (c) 2026 Matt Pocock)

아래 9종은 [mattpocock/skills](https://github.com/mattpocock/skills)에서 vendoring했다.
업스트림 커밋 핀: `694fa30311e02c2639942308513555e61ee84a6f` (2026-06-10).
채택 기준: 2026-06-12 감사 — A급 1(git-guardrails-claude-code: denylist의 PreToolUse hook
기계화) + B급 5(grill-with-docs·prototype·improve-codebase-architecture·zoom-out·handoff) +
집필 3부작(writing-fragments·writing-beats·writing-shape — 업스트림 in-progress 단계,
오너 결정으로 채택·자체 보강 예정). grill-me는 본 pack의 상위판(work 앵커 제작)이 이미
존재해 업스트림판을 덮지 않는다. tdd·diagnose는 superpowers 채택분과 중복이라 불채택.
