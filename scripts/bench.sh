#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${ANTHROPIC_API_KEY:-}" && -z "${CLIPT9N_BENCH_CONFIG:-}" ]]; then
  echo "Set ANTHROPIC_API_KEY or CLIPT9N_BENCH_CONFIG before running benchmarks." >&2
  exit 2
fi

bin="${CLIPT9N_BIN:-target/release/clipt9n}"
out_dir="docs/benchmarks"
mkdir -p "$out_dir"
date_slug="$(date +%Y-%m-%d)"
out="$out_dir/${date_slug}.md"

snippets=(
  "Hello, world."
  "Please review the attached invoice and send feedback by Friday."
  "Guten Tag, ich moechte den Termin auf morgen verschieben."
  "Bu metni daha resmi ve kisa hale getirir misin?"
  "Here is a markdown list:\n- first item\n- second item"
  "The API request failed because the token expired after deployment."
  "Ich brauche eine kurze Zusammenfassung fuer das Standup."
  "Lutfen bu cumledeki yazim hatalarini duzelt."
  "Translate this code comment without touching \`snake_case\` identifiers."
  "A longer paragraph about product onboarding, error recovery, and user trust that should still feel quick."
  "The customer said the onboarding screen felt confusing, but the core workflow was useful."
  "Bitte formuliere diese Nachricht diplomatischer, ohne die Aussage zu veraendern."
  "Toplanti notlarini tek cumlelik bir ozet haline getir."
  "Preserve URLs like https://example.com and email names@example.com."
  "A sentence with smart quotes: “Hello” and guillemets: «bonjour»."
  "Short"
  "This is already English."
  "Das ist bereits Deutsch."
  "Bu zaten Turkce."
  "Final representative snippet for p95 measurement across normal text."
)

durations=()
cargo build --release >/dev/null

for i in "${!snippets[@]}"; do
  start_ns="$(date +%s%N)"
  CLIPT9N_TEST_INPUT="${snippets[$i]}" CLIPT9N_TEST_PRINT_RESULT=1 "$bin" --translate-to=de ${CLIPT9N_BENCH_CONFIG:+--config "$CLIPT9N_BENCH_CONFIG"} >/dev/null
  end_ns="$(date +%s%N)"
  ms=$(((end_ns - start_ns) / 1000000))
  durations+=("$ms")
  printf 'snippet %02d: %sms\n' "$((i + 1))" "$ms"
done

sorted="$(printf '%s\n' "${durations[@]}" | sort -n)"
p50="$(printf '%s\n' "$sorted" | awk 'NR==10 {print}')"
p95="$(printf '%s\n' "$sorted" | awk 'NR==19 {print}')"

{
  echo "# clipt9n latency benchmark - ${date_slug}"
  echo
  echo "- Binary: \`$bin\`"
  echo "- Provider: Anthropic Haiku 4.5 or configured equivalent"
  echo "- Network: record Wi-Fi/Ethernet/VPN status here before committing"
  echo
  echo "| Metric | Target | Actual |"
  echo "|---|---:|---:|"
  echo "| p50 | <800 ms | ${p50} ms |"
  echo "| p95 | <2000 ms | ${p95} ms |"
  echo
  echo "| Sample | Duration |"
  echo "|---:|---:|"
  for i in "${!durations[@]}"; do
    echo "| $((i + 1)) | ${durations[$i]} ms |"
  done
} > "$out"

echo "wrote $out"
