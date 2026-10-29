#!/bin/zsh
set -euo pipefail

script_dir="${0:A:h}"
project_dir="${script_dir:h}"
output_dir="$project_dir/results/power"
audio_dir="$project_dir/test-data/aihub-132-it-S000009/audio/D26/G02/S000009"
timestamp="$(date +%Y%m%d-%H%M%S)"
run_dir="$output_dir/$timestamp"
server_pid=""
power_pid=""

mkdir -p "$run_dir"

cleanup() {
  if [[ -n "$power_pid" ]] && kill -0 "$power_pid" 2>/dev/null; then
    sudo kill -INT "$power_pid" 2>/dev/null || true
    wait "$power_pid" 2>/dev/null || true
  fi
  if [[ -n "$server_pid" ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill -INT "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

measure_command() {
  local label="$1"
  shift
  local power_file="$run_dir/${label}-powermetrics.txt"
  local time_file="$run_dir/${label}-time.txt"

  sudo powermetrics \
    --samplers cpu_power,gpu_power,ane_power \
    --sample-rate 500 \
    --buffer-size 1 \
    --output-file "$power_file" &
  power_pid="$!"
  sleep 1

  /usr/bin/time -l -o "$time_file" "$@"

  sudo kill -INT "$power_pid"
  wait "$power_pid" || true
  power_pid=""
}

cd "$project_dir"

echo "[1/4] 관리자 권한 확인"
sudo -v

echo "[2/4] 15초 idle 전력 기준선 측정"
sudo powermetrics \
  --samplers cpu_power,gpu_power,ane_power \
  --sample-rate 500 \
  --sample-count 30 \
  --buffer-size 1 \
  --output-file "$run_dir/idle-powermetrics.txt"

echo "[3/4] Apple SpeechTranscriber 전체 측정"
measure_command apple \
  "$project_dir/.work/legacy-swift/swift-build/release/grove-apple-benchmark" \
  --audio-dir "$audio_dir" \
  --output "$run_dir/apple.jsonl" \
  --locale ko-KR

sleep 10

echo "[4/4] WhisperKit 서버 로드 후 전체 측정"
if lsof -nP -iTCP:50060 -sTCP:LISTEN >/dev/null 2>&1; then
  echo "포트 50060이 이미 사용 중입니다. 기존 WhisperKit 서버를 종료하고 다시 실행하세요." >&2
  exit 1
fi

whisperkit-cli serve \
  --model large-v3-v20240930_626MB \
  --language ko \
  --host 127.0.0.1 \
  --port 50060 \
  --concurrent-worker-count 1 \
  --chunking-strategy none \
  >"$run_dir/whisperkit-server.log" 2>&1 &
server_pid="$!"

for _ in {1..240}; do
  if nc -z 127.0.0.1 50060 2>/dev/null; then
    break
  fi
  if ! kill -0 "$server_pid" 2>/dev/null; then
    echo "WhisperKit 서버 시작에 실패했습니다. 로그: $run_dir/whisperkit-server.log" >&2
    exit 1
  fi
  sleep 1
done

if ! nc -z 127.0.0.1 50060 2>/dev/null; then
  echo "WhisperKit 서버 준비 시간이 240초를 초과했습니다." >&2
  exit 1
fi

measure_command whisperkit \
  python3 "$project_dir/scripts/run_whisper.py" \
  --audio-dir "$audio_dir" \
  --output "$run_dir/whisperkit.jsonl"

kill -INT "$server_pid"
wait "$server_pid" || true
server_pid=""

sudo chown -R "$(id -u):$(id -g)" "$run_dir"
printf '%s\n' "$run_dir" >"$output_dir/latest-run.txt"

echo "측정 완료: $run_dir"
echo "이 터미널은 닫아도 됩니다. 결과는 Codex가 이어서 분석할 수 있습니다."
