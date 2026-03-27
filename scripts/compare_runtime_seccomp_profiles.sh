#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage:
  compare_runtime_seccomp_profiles.sh [root_dir] [flavor]
  compare_runtime_seccomp_profiles.sh [root_dir] --all-flavors

Examples:
  compare_runtime_seccomp_profiles.sh /tmp/nexus-seccomp-compare debian_ubuntu
  compare_runtime_seccomp_profiles.sh /tmp/nexus-seccomp-compare --all-flavors
EOF
}

ROOT_DIR="${1:-/tmp/nexus-runtime-seccomp-compare}"
MODE="${2:-auto}"
BASELINE_DIR="$ROOT_DIR/baseline"
REPORT_PATH="$ROOT_DIR/baseline_report.txt"

if [[ "$ROOT_DIR" == "--help" || "$ROOT_DIR" == "-h" || "$MODE" == "--help" || "$MODE" == "-h" ]]; then
    usage
    exit 0
fi

mkdir -p "$ROOT_DIR"

run_compare() {
    local flavor="$1"
    local profiles_path="$ROOT_DIR/seccomp_profiles_${flavor}.json"

    cargo run -q --manifest-path /home/fmy/Nexus_OJ/NexusCode/Cargo.toml \
        -p nexus-runtime --bin seccomp_profiles -- --json --flavor="$flavor" >"$profiles_path"

    python3 - "$REPORT_PATH" "$profiles_path" <<'PY'
import json
import sys
from pathlib import Path

report_path = Path(sys.argv[1])
profiles_path = Path(sys.argv[2])

policy_for_label = {
    "cpp": "cpp_native_default",
    "rust": "rust_native_default",
    "wasmtime": "wasm_default",
}

baseline = {}
current = None
for raw_line in report_path.read_text().splitlines():
    line = raw_line.strip()
    if not line:
        continue
    if line.startswith("[") and line.endswith("]"):
        current = line[1:-1]
        baseline[current] = {}
        continue
    if current is None:
        continue
    if ":" not in line:
        continue
    key, value = line.split(":", 1)
    baseline[current][key.strip()] = [item for item in value.strip().split() if item]

profiles = json.loads(profiles_path.read_text())

first_profile = next(iter(profiles.values()), {})
flavor = first_profile.get("flavor", "unknown")
print(f"# flavor: {flavor}")
print()

for label, policy in policy_for_label.items():
    sampled = baseline[label]
    profile = profiles[policy]

    steady = set(sampled.get("steady", []))
    normalized_sampled = set(sampled.get("normalized", []))
    syscalls = set(profile["syscalls"])
    normalized_profile = set(profile["normalized"])

    print(f"[{label} -> {policy}]")
    print(
        "steady_missing: "
        + " ".join(sorted(steady - syscalls))
    )
    print(
        "steady_extra:   "
        + " ".join(sorted(syscalls - steady))
    )
    print(
        "norm_missing:   "
        + " ".join(sorted(normalized_sampled - normalized_profile))
    )
    print(
        "norm_extra:     "
        + " ".join(sorted(normalized_profile - normalized_sampled))
    )
    print()
PY
}

if ! bash /home/fmy/Nexus_OJ/NexusCode/scripts/collect_runtime_syscall_baseline.sh "$BASELINE_DIR" | tee "$REPORT_PATH" >/dev/null; then
    echo "failed to collect runtime syscall baseline" >&2
    echo "hint: run this script on a host where strace/ptrace is allowed, then compare again" >&2
    exit 1
fi

if [[ "$MODE" == "--all-flavors" ]]; then
    for flavor in generic debian_ubuntu arch rhel_like; do
        echo "===== flavor: $flavor ====="
        run_compare "$flavor"
    done
else
    run_compare "$MODE"
fi
