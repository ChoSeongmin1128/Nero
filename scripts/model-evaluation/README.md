# 모델 평가

이 도구는 로컬 모델 후보를 같은 입력으로 비교하기 위한 연구용 도구다. 제품의 지원 모델 목록이나 실제 하드웨어 검증 결과가 아니다. 실행기는 현재 macOS/Linux의 POSIX 프로세스 그룹을 사용하며 Windows 실행은 구현되지 않았다. `models.json`은 후보와 출처, 주장 및 이 경로의 제약을 기록한다. `documented`는 문서화된 기능, `claimed`는 모델 제공자의 주장, `verified`는 이 평가 경로에서 확인된 기능, `unverified`는 미확인을 뜻한다. 실행 성공이나 품질을 보장하지 않는다.

## 준비

- 도구 준비와 실제 모델 실행 준비는 별개다. 예시 설정에는 실제 모델/런타임/음성/정답 경로가 없으므로 그대로 실행 가능한 평가 자료가 아니다. 경로와 출처를 채운 후 plan/doctor를 통과시키고 실제 추론 및 채점 결과를 확인해야 한다. 합성 테스트 통과는 이 준비나 모델 품질 검증을 대신하지 않는다.
- 모델과 런타임은 사용자가 별도로 준비한다. 모델 다운로드, 패키지 설치, 외부 AI 호출 및 비용 발생을 자동으로 수행하지 않는다.
- 모델 파일은 기존 캐시의 절대 경로를 지정해 모든 실행에서 재사용한다. 실행마다 복사하지 않는다.
- 오디오, 정답 전사, 용어와 모델 경로는 비공개 설정에 직접 넣는다. 설정과 결과는 저장소에 올리지 않는다. 예시 경로는 실제 파일이 아니다.
- `models`의 사용자 지정 ID는 `extends`에 기존 레지스트리 ID를 지정하고 `format`, `variant`, `source`를 덮어쓸 수 있다. `source`는 레지스트리 출처와 병합된다. 변환 GGUF는 실제 `conversion_repository`, 불변 `revision`, `file`, `sha256`을 직접 확인해 지정한다. Qwen의 원본 저장소 리비전은 변환 파일의 출처를 대신하지 않는다. 다른 크기나 정밀도의 Gemma는 별도 프로필에서 실제 `repository`, `revision`, `file`, `sha256`을 모두 덮어쓰고 기존 파일에만 해당하는 `manifest`, `url`, `license`도 실제 값이나 `null`로 덮어쓴다. Qwen 원본 모델의 `base_license`와 사용자가 선택한 변환본의 `conversion_license`는 별개이며, 변환본 정보는 사용자가 확인한다. 예시의 `REPLACE_WITH_...`는 실제 값이 아니며 교체해야 한다.
- MOSS는 기존 고정 모델 및 native harness와 mono 16 kHz WAV가 필요하다. Nemotron Mini는 fused `fused-64e-w4a16-g64`를 선택한다. text-only `backbone-64e-w4a16-g64`는 음성 모델이 아니다. Mini 실행에는 Linux/CUDA, vLLM Marlin 및 nemo_speechlm이 필요하며 Mac과 이 평가 실행 경로는 실측하지 않았다. 기록된 리비전은 저장소 커밋이며 로컬 가중치가 그 커밋과 일치하는지는 별도 검증하지 않았다. 모델 카드의 10GB는 가중치 크기이지 최대 RAM 또는 VRAM이 아니다.
- Qwen ASR은 로컬 `qwen-asr`를 사용한다. 교정용 GGUF는 로컬 llama-server와 호환되어야 한다. 각 모델의 타임스탬프, 화자, 문맥 기능은 따로 확인한다. CTC 타임스탬프와 DFlash 또는 draft 모델은 서로 다른 기능이다.
- 교정 llama-server는 데스크톱 요약 실행과 같은 조건으로 시작한다. 사고 모드 꺼짐(`--reasoning off`, `--jinja`, `enable_thinking: false` 서버 기본값과 요청 값), 문맥 8192, 병렬 1, 문맥 이동 없음이다. 요청 기본값은 temperature 0, top_k 1, seed 0, 후보 ID와 판정만 허용하는 strict JSON schema, 출력 상한 768이며 출력 상한은 8192보다 작아야 한다. 스레드와 배치 크기 같은 앱 자원 조정값은 적용하지 않는다. 빈 본문 응답은 실패로 남기고 추론 텍스트 길이를 기록한다.
- Qwen ASR은 runtime의 `dtype`으로 `bfloat16`, `float16`, `float32` 중 하나를 지정할 수 있다. 지정하지 않으면 런타임 기본값을 사용한다. 모델 제공자의 예시와 평가는 `bfloat16`을 사용한다.
- Qwen ASR의 `decoding.chunk_seconds`(1~1200)를 지정하면 qwen-asr의 저에너지 경계 분할로 입력을 나눠 각 구간을 전사하고 공백으로 잇는다. 각 구간이 입력의 해당 샘플과 겹침이나 빈 곳 없이 일치하는지 확인하며(0.5초 미만 구간의 런타임 0 채움은 허용하고 기록) 구간별 EOS는 확인하지 않는다. 지정하지 않으면 런타임이 20분 이하 입력을 한 번에 처리한다. 두 방식은 별도 실험으로 비교한다.
- 분할 옵션은 `chunk_seconds`와 함께만 쓴다. `chunk_boundary`는 `energy`(기본, qwen-asr 경계)나 `pause`다. `pause`는 목표 길이의 절반 지점부터 목표 + 3초까지에서 20ms 프레임 에너지가 하위 20%의 1.5배와 중앙값 -12dB보다 모두 낮은 구간이 가장 길게 이어지는 곳의 가운데를 자른다. `chunk_pad_seconds`(0~5)는 각 구간 앞뒤에 무음을 붙이고, `chunk_gain_db`(-40~-6)는 0.5초 창마다 RMS를 그 값으로 맞춘다(최대 +20dB, -55dB 미만 창은 그대로). 커버리지 검사는 무음과 음량 처리 전의 원래 구간으로 한다.
- 교정 요청의 `decoding`에는 `temperature`, `top_p`, `top_k`, `min_p`, `presence_penalty`, `repeat_penalty`, `seed`를 지정할 수 있다. 지정하지 않은 값은 위의 결정적 기본값을 쓴다. 모델 제공자 권장값을 쓸 때는 seed를 바꿔 반복 실행한다.
- Qwen ASR의 추론 배치는 1로 제한한다. Mini 서버는 모델 카드 예시의 `gpu-memory-utilization=0.9`, `max-model-len=8192`를 사용하며 실제 입력과 출력이 그 길이에 들어가야 한다. `trust-remote-code`는 자동 활성화하지 않아 모델 카드의 해당 설정이 필요한 환경은 실행되지 않을 수 있다. 준비된 로컬 런타임과의 호환성 검증이 필요하다.

## 어댑터 출력

| 경로 | 평가기에 전달되는 출력 |
|---|---|
| MOSS | 텍스트와 원본 화자 구간. 입력 전체와 EOS 증거를 검사한다. |
| Qwen ASR | 텍스트와 선택적 문맥. 타임스탬프와 화자 구간은 반환하지 않으며 입력 전체 처리 완료 증거가 없어 잠정 결과다. |
| Nemotron Mini | 텍스트만 반환하는 잠정 결과. 이 경로는 원본 화자 분리, CTC 타임스탬프와 문맥 입력을 구현하지 않는다. 원시 제어 또는 화자 마커가 있으면 텍스트 채점과 후속 교정을 생략하고 원본 출력을 보존한다. |
| GGUF 교정 | 지정된 후보별 수락/유지/불확실 판정으로 적용한 텍스트. 후보가 없으면 실행하지 않는다. |

## 실행

`benchmark.example.json`을 저장소 밖 비공개 경로에 복사해 실제 경로를 채운다. `output_root`는 Git에서 제외되는 비공개 `.work` 경로로 지정한다. `models`에는 선택한 로컬 모델과 실행 파일의 절대 경로를 넣고, `experiments`에서 ASR과 교정 조합을 선택한다. `baseline`은 교정에 사용할 ASR 실험 ID다. 필요 없는 모델과 실험은 제거한다. 실행은 명시적으로 요청한 경우에만 수행한다.

각 case의 `reference`는 `{"text": "...", "segments": [{"start": 0.0, "end": 1.0, "speaker": "..."}], "entities": ["평가 라벨"]}` 형식의 비공개 JSON이며 채점기에만 전달한다. `context`는 선택 사항이며 `{"terms": ["..."], "attendees": ["..."], "corrections": [{"source": "...", "replacement": "...", "kind": "...", "context_cues": ["..."]}]}` 형식이다. 예시의 경로와 값은 실제 입력으로 교체해야 한다. `metrics.collar`와 `overlap_policy`는 화자 점수의 평가 조건이다.

```bash
python3 scripts/model-evaluation/eval.py plan --config /absolute/private/config.json
python3 scripts/model-evaluation/eval.py doctor --config /absolute/private/config.json
python3 scripts/model-evaluation/eval.py run --config /absolute/private/config.json
python3 scripts/model-evaluation/eval.py report --run-dir /absolute/private/run-directory
```

plan과 doctor는 사전 조건을 확인하며 모델 바이트를 읽거나 추론을 시작하지 않는다. doctor는 입력 WAV의 메타데이터와 해시 및 정답/context 형식을 읽는다. 준비 완료 표시는 실제 모델 런타임 지원의 증거가 아니다. run은 로컬 모델 바이트의 지문을 모델별로 한 번 계산한다. run은 지정된 오디오와 텍스트를 로컬 모델에 전달한다. 결과에는 민감한 전사와 경로가 포함될 수 있으므로 Git 추적 밖의 비공개 경로에서 관리한다.

## 비교 원칙

- 같은 오디오에서 ASR baseline과 context-only를 비교하고, 동일한 ASR 출력에 교정 단계를 적용한 조합을 따로 비교한다. 용어 입력과 교정 후보를 고정한다.
- 엔터티 지표는 정답에 등장하는 고유 평가 라벨의 NFC 부분 문자열 존재 여부를 센다. 단어 경계와 반복 횟수는 구분하지 않으므로 라벨을 신중히 선택한다.
- ASR과 교정은 서로 다른 단계이므로 단일 점수로 서로 순위를 매기지 않는다. 교정 전후는 같은 원본 전사와 정답을 사용한다. 정답 전사는 채점에만 쓰고 모델에는 전달하지 않는다.
- 지원하지 않는 기능, 증거가 없는 타임스탬프 및 실패한 실행은 미지원 또는 미측정으로 남긴다. 점수 0으로 바꾸지 않는다.
- 각 실험은 새 프로세스에서 순차 실행한다. 경과 시간은 모델 로딩, 서버 시작, 추론, 종료를 포함하며 입력/모델 해시와 채점은 제외한다. 모델 단독 추론 시간과 CPU RAM/GPU VRAM은 측정하지 않는다.
- 교정 후보가 하나도 일치하지 않으면 모델 실행을 생략하고 미채점으로 기록한다.
- 정확한 CER 계산에서 비교 격자가 2,000,000셀을 초과하면 선택적 RapidFuzz가 필요하다. 없으면 해당 지표는 미지원으로 기록한다.
- 단일 결과는 실험 결과일 뿐이며 장시간 녹음, 한국어 품질, 실제 기기와 제품 배포에 대한 검증을 대신하지 않는다.

## 검증

```bash
python3 -B -m unittest discover -s scripts/model-evaluation/tests -v
```

테스트는 합성 입력으로 도구 계약을 확인한다. 실제 모델, 사용자 녹음 또는 하드웨어 실행을 검증하지 않는다. 같은 검사를 별도 경량 GitHub Actions 작업으로 실행하며 앱 빌드와 모델 다운로드를 시작하지 않는다. 시간 초과, Ctrl-C와 SIGTERM은 평가가 생성한 프로세스 그룹만 정리하고 로그와 완료된 영수증을 보존한다.
