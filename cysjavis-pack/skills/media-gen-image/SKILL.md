---
name: media-gen-image
description: 텍스트 설명에서 이미지를 생성하는 하위 스킬 — 사용자의 거친 의도를 모델 최적 프롬프트로 재작성한 뒤 fal.ai 이미지 모델(nano-banana-pro 등)로 생성한다. media-gen 이미지 관문. "이미지 생성 / text to image / 프롬프트 재작성 / nano banana" 맥락에서 발동.
---

# media-gen-image

설명에서 이미지를 만든다. 핵심은 **프롬프트 재작성** — 사용자의 거친 한 줄을 모델이 가장 잘
이해하는 프롬프트로 Claude가 다시 쓴 뒤 생성한다(원본 영상의 핵심 기법).

## 절차

1. **의도 수집** → 검증: 사용자의 대략 의도 + 모델·비율·해상도 선호를 받는다.
2. **프롬프트 재작성** → 검증: 거친 의도를 *모델 최적* 프롬프트로 확장(피사체·스타일·조명·
   구도·디테일). 재작성 결과를 사용자에게 보여주고(투명성) 생성.
3. **모델·파라미터** → 검증: 기본 `fal-ai/nano-banana-pro`(고품질). 라이브 스키마 조회 후
   호출 — 이 모델은 `aspect_ratio`(예: `1:1`,`16:9`) + `resolution`(`1K/2K/4K`) + `output_format`,
   `seed`. **스키마 하드코딩 금지.**
4. **비용·생성** → 검증: 유료면 `[[cost-preview-confirm]]`. 큐 제출 → 폴링 → 결과 URL GET →
   `media/images/`에 저장. 응답 `images[].url`.
5. **자기교정** → 검증: 거부·실패(세이프티·파라미터 오류)면 사유를 읽고 프롬프트/파라미터를
   교정해 1~2회 재시도.

## 출력 계약

`media/images/NN.<ext>` + 매니페스트 항목(모델경로·재작성 프롬프트·seed·aspect·resolution·실비용).
상위 `[[media-gen]]`로 반환. 편집·참조가 필요하면 `[[media-gen-edit]]`, 영상화는 `[[media-gen-video]]`.
