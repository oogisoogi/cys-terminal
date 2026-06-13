---
name: media-gen-video
description: 이미지를 시네마틱 영상 클립(B-roll)으로 애니메이트하는 하위 스킬 — fal.ai 영상 모델(Kling·Seedance 등)로 이미지→영상, 초당 종량제라 길이·비용을 정밀 제어한다. media-gen 영상 관문. "이미지를 영상으로 / B-roll 생성 / image to video / kling seedance" 맥락에서 발동.
---

# media-gen-video

이미지를 움직이는 클립으로 만든다(B-roll). **초당 과금**이라 길이가 곧 비용 — 정밀하게 잡는다.

## 절차

1. **입력 준비** → 검증: 시작 프레임 이미지를 URL로(`image_url`). 로컬이면 업로드 후 URL.
2. **모델 선택** → 검증: 기본 `fal-ai/kling-video/.../image-to-video`(저가 베스트) 또는
   `bytedance/seedance-2.0/image-to-video`(고품질·고가). 트레이드오프(비용/품질)를 명시하고 결정.
   라이브 스키마 조회(파라미터·제약은 모델별).
3. **길이·비용** → 검증: `duration`을 *필요한 만큼만*(예: 3~5초). **유료 종량제라 실행 전
   `[[cost-preview-confirm]]` 필수**(fal 추산 API 활용 — 5초 vs 10초가 수 달러 차이).
4. **생성·자기교정** → 검증: 큐 제출 → 폴링 → 결과 영상 URL → `media/broll/`에 저장. 입력
   이미지 용량/포맷 제약(모델별 — 예 Seedance 30MB) 초과면 압축·변환 후 재실행(자기교정).
5. **품질 점검** → 검증: 해상도·프레임레이트가 본 영상과 정합 가능한지(부족하면 `[[media-gen-upscale]]`).

## 출력 계약

`media/broll/NN.mp4` + 매니페스트(모델경로·입력이미지·프롬프트·duration·실비용). 상위
`[[media-gen]]`로 반환 → `[[video-stitch-broll]]`이 타임라인에 배치.
