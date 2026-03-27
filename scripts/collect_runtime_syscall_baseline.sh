#!/usr/bin/env bash
set -euo pipefail

require_tool() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "missing required tool: $1" >&2
        exit 1
    fi
}

require_tool strace
require_tool rustup
require_tool /usr/bin/g++
require_tool python3

ROOT_DIR="${1:-/tmp/nexus-runtime-syscall-baseline}"
mkdir -p "$ROOT_DIR"

CPP_SRC="$ROOT_DIR/cpp_baseline.cpp"
CPP_BIN="$ROOT_DIR/cpp_baseline"
CPP_TRACE="$ROOT_DIR/cpp_baseline.strace"

RUST_SRC="$ROOT_DIR/rust_baseline.rs"
RUST_BIN="$ROOT_DIR/rust_baseline"
RUST_TRACE="$ROOT_DIR/rust_baseline.strace"

WASM_SRC="$ROOT_DIR/wasm_baseline.rs"
WASM_BIN="$ROOT_DIR/wasm_baseline.wasm"
WASM_TRACE="$ROOT_DIR/wasm_baseline.strace"

cat >"$CPP_SRC" <<'EOF'
#include <iostream>
int main() {
    int a = 0, b = 0;
    if (!(std::cin >> a >> b)) return 0;
    std::cout << (a + b) << "\n";
    return 0;
}
EOF

cat >"$RUST_SRC" <<'EOF'
use std::io::{self, Read};

fn main() {
    let mut s = String::new();
    io::stdin().read_to_string(&mut s).unwrap();
    print!("{}", s);
}
EOF

cat >"$WASM_SRC" <<'EOF'
use std::io::{self, Read};

fn main() {
    let mut s = String::new();
    io::stdin().read_to_string(&mut s).unwrap();
    print!("{}", s);
}
EOF

/usr/bin/g++ -std=c++20 -O2 -pipe -o "$CPP_BIN" "$CPP_SRC"
"$(rustup which rustc)" -O -o "$RUST_BIN" "$RUST_SRC"
"$(rustup which rustc)" --target wasm32-wasip1 -O -o "$WASM_BIN" "$WASM_SRC"

trace_run() {
    local trace_path="$1"
    shift
    if ! printf '1 2\n' | strace -f -qq -o "$trace_path" "$@" >/dev/null 2>/dev/null; then
        echo "failed to collect syscall baseline with strace: $*" >&2
        echo "this usually means ptrace/strace is restricted in the current environment" >&2
        exit 1
    fi
}

trace_run "$CPP_TRACE" "$CPP_BIN"
trace_run "$RUST_TRACE" "$RUST_BIN"
trace_run "$WASM_TRACE" /root/.wasmtime/bin/wasmtime run -W max-memory-size=268435456 -W timeout=1000ms "$WASM_BIN"

python3 - "$CPP_TRACE" "$RUST_TRACE" "$WASM_TRACE" <<'PY'
import re
import sys
from pathlib import Path

BOOTSTRAP_ONLY = {"execve"}
ALIASES = {
    "file_stat": {"fstat", "newfstat", "newfstatat", "statx"},
    "process_clone": {"clone", "clone3", "vfork"},
    "signal_runtime": {"rt_sigaction", "rt_sigprocmask", "sigaltstack", "rt_sigreturn"},
    "scheduler_affinity": {"sched_getaffinity", "sched_yield"},
}

def collect(path: Path):
    calls = set()
    for line in path.read_text(errors="ignore").splitlines():
        match = re.match(r"\d+\s+([a-zA-Z_][a-zA-Z0-9_]*)\(", line)
        if match:
            calls.add(match.group(1))
    return sorted(calls)

def steady_state(calls):
    return sorted(call for call in calls if call not in BOOTSTRAP_ONLY)

def normalize(calls):
    raw = set(calls)
    normalized = set()
    for call in raw:
        matched = False
        for group, members in ALIASES.items():
            if call in members:
                normalized.add(group)
                matched = True
        if not matched:
            normalized.add(call)
    return sorted(normalized)

labels = ["cpp", "rust", "wasmtime"]
for label, raw in zip(labels, sys.argv[1:]):
    calls = collect(Path(raw))
    print(f"[{label}]")
    print("raw:       " + " ".join(calls))
    print("steady:    " + " ".join(steady_state(calls)))
    print("normalized:" + " ".join(normalize(calls)))
    print()
PY
