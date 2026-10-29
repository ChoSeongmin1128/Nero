# 검수 데이터·버전 계약

상태: **Planned / 논리 스키마 초안, 미구현** · 버전: 0.1 · 2026-09-03

AI-assisted 문서입니다. 아래 새 필드/검증기/저장 형식은 아직 구현된 API가 아닙니다. 기존 `schemaVersion`을 문서 작성만으로 변경하지 않습니다. [동작 사양](review-mode-spec.md), [내보내기](dataset-export.md)와 함께 읽으십시오.

## 현재 구조와 이관 제약

beta.8의 문서 schema5에는 **화자 배정 확인용** 근거와 undo 명령이 추가됐습니다.
이는 아래의 전체 annotation/correction-head/audio-hash/snapshot 계약을 구현한 것이 아닙니다.
[현재 UI 확인 계약](transcript-library-ux.md)을 별도로 참고하십시오.

- [DocumentUtterance](../legacy/swift/Sources/GroveApp/TranscriptDocument.swift#L11)는 Double 시각, rawText/editedText, 원래 ID·채널·군집·단일 parent ID를 가집니다. `displayedText`는 editedText 우선이며 **그 수정의 목적을 기록하지 않습니다**.
- [TranscriptDocument](../legacy/swift/Sources/GroveApp/TranscriptDocument.swift#L82)는 schema 3–4를 읽고 `revisionID`, 실제 엔진 정보, undo/redo를 보관합니다. [commit](../legacy/swift/Sources/GroveApp/TranscriptDocument.swift#L234)은 매 편집마다 `revisionID`를 바꾸지 않습니다.
- [InferenceResult](../legacy/swift/Sources/GroveInference/InferenceOutput.swift#L66)의 jobID/기계 초안과 [단일 채널 변환](../legacy/swift/Sources/GroveApp/InferenceDocumentBridge.swift#L22)의 document revisionID는 현재 공유될 수 있습니다. 이를 새 교정 커밋 ID나 dataset snapshotID라고 해석하지 않습니다.
- [Storage](../legacy/swift/Sources/GroveApp/TranscriptDocumentStorage.swift#L35)는 이전 전사 archive와 원시 engine result를 따로 보존합니다. `ArchivedTranscript.id` 역시 데이터셋 snapshotID가 아닙니다.

## 분리할 객체와 식별자

| 객체 | 필수 논리 필드 | 불변/변경 규칙 |
|---|---|---|
| AudioAsset | audioAssetID, originalSHA256, frameCount/duration, channels, timebaseID, 원본 참조, 원본 계보 그룹 | 원본 바이트·해시 불변. 교체 파일을 같은 자산으로 가장하지 않음 |
| Timebase | timebaseID, audioAssetID, channelID, ticksPerSecond, originTick, 변환/원본 매핑 정보 | 양의 정수 tick 기준. 채널·디코드 기준 변경 시 새 시간기준 |
| ModelRun | modelRunID, source hash/channel/timebase, 모델·runtime 버전, 가중치 해시, 요청/실제 엔진·설정, 불변 출력 참조 | 실행별 새 ID. 실제 출력의 근거이며 교정/검수 상태를 소유하지 않음 |
| AnnotationDocument | annotationDocumentID, meetingID, 원본 참조, baseModelRunIDs[], 현재 교정 head | 일반 대화/검수 화면의 공통 문서 |
| CorrectionRevision | correctionRevisionID, parentRevisionID, commandID, annotations, reviews, undo/redo, 작성 시각 | 성공 커밋마다 새 ID. undo도 새 커밋이지만 이전 의미 상태를 복원 |
| TranscriptAnnotation | annotationID, channel/timebase, startTick/endTick, verbatimText 후보·역할, meetingSpeakerID, provenance, 삭제 상태/사유 | 전사 문장/발화 단위; 모델 원문은 참조로 보존 |
| SpeakerActivity | activityID, channel/timebase, startTick/endTick, meetingSpeakerID 또는 불명확, provenance | 실제 말소리 활동 단위. 전사 구간으로 자동 대체하지 않음 |
| ReviewEvidence | reviewEvidenceID, 대상·축·범위, decision, binding, reviewer, reviewedAt, reason | 확인한 입력에 바인딩된 증거 사건. 과거 증거는 재작성하지 않음 |
| CoverageReview | coverageID, purpose, channel/timebase, 범위, reviewEvidenceID | 전사 완전성/화자 활동 완전성을 별도로 저장. 침묵도 검수 가능 |
| DatasetSnapshot | snapshotID, correctionRevisionID들, 선택 목적/범위, 증거·지침·승인 버전, 내용 해시, 포함/제외 명세 | 생성 후 불변. 재내보내기나 재학습용 수정은 새 snapshot |

ID가 우연히 같은 문자열이어도 역할을 혼용하지 않습니다. 하나의 병합 항목이 여러 ModelRun/원본 항목에 걸칠 수 있으므로 원본 참조는 목록입니다. 수동 추가에 가짜 modelRunID를 만들지 않습니다.

## 시간과 출처

- 정본 범위는 동일 시간기준의 정수 `[startTick, endTick)`이며 `0 ≤ startTick < endTick ≤ audioEndTick`입니다. 모호한 끝을 임의 생성하지 않습니다.
- 화면 소수 둘째 자리·파형 픽셀은 표시값입니다. 드래그/숫자 입력은 명시적 tick 변환 규칙을 거쳐 한 번 커밋합니다. TSV/RTTM 초 변환의 반올림 규칙도 버전으로 고정합니다.
- 재샘플링된 분석 파일은 원본 음성 참조·변환 버전·frame mapping을 가집니다. 16kHz 분석 프레임을 원본 다른 sample rate의 frame으로 오인하지 않습니다.
- 옛 두 채널 녹음은 채널별 시간기준을 보존합니다. 검증되지 않은 오프셋으로 공동 타임라인/동일 화자를 만들지 않습니다.
- `provenance`는 `kind: model/manual/split/merge`, `parentAnnotationIDs[]`, `sourceReferences[]`, `createdByCommandID`를 포함합니다. source reference는 modelRunID+원본 항목 ID/범위 또는 수동 근거를 가리킵니다.
- 병합은 **모든 직접 부모와 전이적 모델 원본**을 추적할 수 있어야 합니다. 기존 parentUtteranceID와 원래 ID를 삭제하거나 새 ID로 일괄 치환하지 않습니다.
- 삭제는 교정본의 tombstone/명령으로 기록합니다. 삭제 전 구간·텍스트·화자·순서가 undo에서 복원되어야 합니다. 모델 원본이나 음성 삭제를 의미하지 않습니다.

## 검수 증거의 바인딩과 유효성

공통 binding은 `audioSHA256 + channelID + timebaseID + labelingGuidelineVersion`을 포함합니다. 여기에 **해당 축에 관련된 입력만** canonical digest로 묶습니다.

| 축 | 추가로 바인딩할 입력 | 그 외 값 변경의 처리 |
|---|---|---|
| text | annotationID, start/end tick, 실제 발화 후보 텍스트와 역할 | 표시 이름/제목 변경으로 무효화하지 않음 |
| speaker | annotationID, start/end tick, 회의 내 화자 ID, 관련 겹침/배정 근거 | 같은 실체의 표시 이름 수정은 유지; 실제 ID 재배정은 무효 |
| timing | annotationID, 시작/끝, 관련 경계·겹침 근거 | 순수 표기 교정은 시간 증거를 자동 폐기하지 않음 |
| activity | activityID, 시작/끝, 화자 ID/불명확, 말소리 종류, 관련 겹침 | 전사 텍스트만 바뀌면 독립 활동 원값은 유지 |
| transcriptCompleteness | 범위, 교차하는 유효 전사 ID·시각·불명확/제외 상태 | 발화 구성·범위·불명확/제외 상태 변화의 교차 범위만 재검수. 순수 단어 교정은 text 축에서 확인하며, 이름/활동 층 변경도 무효화 근거가 아님 |
| activityCompleteness | 범위, 교차하는 활동 ID·시각·화자 ID·말소리 종류·불명확/제외 상태 | 확인된 침묵 포함. 전사 텍스트 수정/삭제만으로 활동 또는 무음을 승인하지 않음 |

저장 상태는 `unreviewed / verified / uncertain`입니다. 과거 verified 증거의 binding이 현재와 다르면 유효 상태는 unreviewed(`stale` 사유)로 평가합니다. `reviewedAt`이나 `model confidence`만으로 verified가 되지 않습니다.

범위 검수는 의존 구간이 바뀐 **교집합만** 분리해 무효화합니다. 영향 밖 범위는 이전 증거에서 파생됨을 기록해 보존합니다. 경계 편집은 old∪new 및 영향 겹침, 삭제는 old 범위만 대상으로 하며, 그 계산 결과도 명령의 변경 집합에 포함합니다. 세부 명령 표는 [사양](review-mode-spec.md#편집-명령재검수undo-계약)을 따릅니다.

회의 제목, 화자 표시 이름, 폴더 이름은 위 내용 digest에서 제외합니다. 외부 사람 프로필의 실명 확인이 필요한 작업은 별도 identity 확인이며 익명 화자 분리 정답과 혼동하지 않습니다.

Undo는 내용과 해당 시점의 증거/유효 상태를 함께 되돌립니다. 단, 그 사이 원본·시간기준·적용 지침이 달라졌다면 옛 증거가 다시 유효해지는 것은 아닙니다. 과거 감사 사건과 이미 생성된 snapshot은 수정하지 않습니다.

## 원자적 명령과 저장 계약

명령 입력은 commandID, expectedCorrectionRevisionID, 대상 IDs, old/new 내용, 변경 사유를 가집니다. 명령 적용 결과는 내용 delta·검수 delta·범위 무효화 delta·undo 정보를 한 새 CorrectionRevision에 담습니다.

- 잘못된 ID/범위/채널/버전·겹침을 제거하는 자동 병합·존재하지 않는 원본 참조를 저장 전에 거절합니다.
- 내용 저장 후 검수 상태를 별도 비동기 쓰기로 갱신하는 구조는 금지합니다.
- 저장 성공 후 UI 발행을 유지합니다. 저장 실패에서 undo 스택만 전진하거나 verified 표시만 남지 않아야 합니다.
- 새 head 활성화는 revision 전제조건을 검사합니다. 재전사/복원/다른 편집으로 head가 바뀌면 입력을 보존하고 충돌을 처리합니다.
- 첫 베타부터 중단 쓰기·디스크 오류·손상 JSON·재시작 복구를 시험합니다. 마지막 정상 커밋/원본 파일을 보존하며 자동 빈 문서 초기화로 덮어쓰지 않습니다.

## 마이그레이션

1. 기존 schema 3/4 원형 JSON과 참조 음성·모델 출력은 먼저 보존합니다. 기존 ID·editedText·undo/redo·엔진 provenance를 새 의미로 덮어쓰지 않습니다.
2. editedText가 있는 항목은 `legacyUnclassified`로 표시하고 기존 화면 내용은 유지합니다. 원음그대로 교정문인지 확인 전에는 ASR 정답 후보로 승인하지 않습니다. editedText가 없어도 rawText가 검수 완료인 것은 아닙니다.
3. 현재 revisionID는 legacy generation ID로 남깁니다. 새 교정 커밋 ID와 모델 실행 참조를 별도로 만들고, 근거를 못 찾은 legacy modelRun 참조는 unresolved로 기록합니다.
4. 기존 분할의 단일 parent 정보를 보존하고 가능한 출처만 이관합니다. 없는 다중 부모·시각 정밀도·검수 이력을 추정해서 채우지 않습니다.
5. Double 시각의 원값을 legacy 메타데이터에 보관하고 정수 변환 규칙/오차를 기록합니다. 변환은 사람 시각 검수로 간주하지 않습니다.
6. 새 검수는 전부 미검수, 학습 승인도 미승인으로 시작합니다. 이관 전후 일반 화면·편집/undo 의미의 동일성과 저장 실패 롤백을 fixture로 확인한 뒤 새 head를 활성화합니다.

## 계약 완료 기준

- 합성 schema3/4 문서, 원문 수정, 다단 분할/병합, 삭제·복구, 한글·영문·줄바꿈이 내용·ID·undo/redo 손실 없이 왕복합니다.
- 모델 재실행·교정 커밋·검수 사건·dataset snapshot의 ID가 독립적으로 추적됩니다.
- 제목/이름만 바뀐 경우 검수가 유지되고, 관련 text/speaker/timing·음성·지침 변경에는 필요한 범위만 무효화됩니다.
- 저장 실패 전후 내용/검수/undo가 혼합 버전이 되지 않습니다. 구현·마이그레이션 테스트 통과 전에는 데이터셋 생성용 완료로 선언하지 않습니다.
