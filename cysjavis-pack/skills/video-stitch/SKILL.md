---
name: video-stitch
description: 아바타 클립·HyperFrames 모션그래픽·생성형 B-roll·자막·배경음악을 하나의 완성 영상으로 합성하는 대표 편집 스킬 — 컴포지팅(아바타 항상 노출)·B-roll 배치·자막 번인을 하위 스킬로 오케스트레이션하고 FFmpeg로 마스터 렌더한다. "영상 합성 / 편집 / 클립 결합 / 컴포지팅 / 자막" 트리거, 또는 youtube-video-pipeline 합성 단계로 발동.
---

# video-stitch

여러 트랙(아바타·모션그래픽·B-roll·자막·음악)을 사람 편집자처럼 하나로 합친다. 모션그래픽
*제작*은 **HyperFrames 공식 `/hyperframes` 스킬**(HTML), B-roll *생성*은 `[[media-gen]]`,
음악은 `[[audio-post]]`가 한다 — 이 스킬은 그 결과들을 **편집·합성**한다.

> 편집은 결정론적 타임라인 작업이지만, B-roll·자막·"아바타 항상 노출" 컴포지팅이 더해지며
> 별개 편집 관심사로 갈라져 대표+하위 구조로 운영한다.

## 하위 스킬 오케스트레이션

1. `[[video-stitch-compositing]]` — 아바타 항상 노출(좌측 카드/둥근 크롭) + 전환·페이싱.
2. `[[video-stitch-broll]]` — `[[media-gen]]`의 생성형 B-roll을 타임라인 적소에 배치.
3. `[[video-stitch-captions]]` — `audio/timestamps.json` 기반 동기 자막 번인.

## 절차

1. **아바타 트랙 결합** → 검증: `avatar/clip-NN.mp4`를 순서대로 concat. 경계가 오디오
   연속과 맞는지(끊김·중복 없음).
2. **그래픽 타임라인** → 검증: 시각 큐별 HyperFrames 블록(`/hyperframes` HTML 렌더)의
   in/out을 타임스탬프에 매핑.
3. **B-roll 배치** → 검증: `[[video-stitch-broll]]` — 생성형 클립을 해당 구간에 삽입(아바타
   노출 규칙과 충돌 없게).
4. **아바타 컴포지팅** → 검증: `[[video-stitch-compositing]]` — 모든 프레임에 아바타 가시,
   둥근 크롭이면 드롭섀도·코너 반경 일관, 전환·여백 프로페셔널.
5. **자막** → 검증: `[[video-stitch-captions]]` — 동기 자막 번인(화면 텍스트는 검증 4관문 대상).
6. **음악 믹스·마스터 렌더** → 검증: `[[audio-post]]`의 음악 베드를 나레이션 아래로 덕킹
   믹스 + 오디오·영상 트랙을 `final/video.mp4`로 렌더. 해상도·fps·싱크 일관.

## 출력 계약

`final/video.mp4` + `final/timeline.json`(그래픽·B-roll·아바타 모드·자막의 in/out·모드 —
검증 단계의 대조 기준). 상위 `[[youtube-video-pipeline]]`로 반환 → `[[video-verify]]` 입력.
