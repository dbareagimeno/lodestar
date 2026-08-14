#!/usr/bin/env bash
set -euo pipefail

lodestar_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$lodestar_root"

gate_contract_static() {
  python3 scripts/check-contract-surface.py
}

gate_contract() {
  gate_contract_static
  cargo test -p lodestar-mcp --bin lodestar-mcp --locked tools_list_lleva
  cargo test -p lodestar-mcp --test descubribilidad --locked schema_declara_todos_los_parametros
  cargo test -p lodestar-mcp --test descubribilidad --locked el_schema_declarado_coincide_con_lo_aceptado
}

gate_dependency_policy() {
  local core_tree workspace_tree forbidden_core forbidden_workspace error_pattern sources matches

  core_tree="$(cargo tree -p lodestar-core --edges normal --prefix none --locked)"
  forbidden_core='^(tokio|rusqlite|libsqlite3-sys|git2|libgit2-sys|notify|notify-debouncer-full|tauri|tauri-build|zip) '
  if printf '%s\n' "$core_tree" | grep -Eq "$forbidden_core"; then
    printf '%s\n' "ERROR: lodestar-core arrastra una dependencia prohibida" >&2
    printf '%s\n' "$core_tree" | grep -E "$forbidden_core" >&2 || true
    return 1
  fi

  workspace_tree="$(cargo tree --workspace --edges normal --prefix none --locked)"
  forbidden_workspace='^(git2|libgit2-sys|lodestar-vcs|zip) '
  if printf '%s\n' "$workspace_tree" | grep -Eq "$forbidden_workspace"; then
    printf '%s\n' "ERROR: el workspace arrastra git/libgit2/ZIP retirados" >&2
    printf '%s\n' "$workspace_tree" | grep -E "$forbidden_workspace" >&2 || true
    return 1
  fi

  error_pattern='"(WORKSPACE_NOT_FOUND|WORKSPACE_RECOVERY_REQUIRED|DOCUMENT_NOT_FOUND|DOCUMENT_ALREADY_EXISTS|AMBIGUOUS_REFERENCE|REVISION_CONFLICT|PLAN_STALE|PLAN_EXPIRED|PERMISSION_DENIED|INVALID_SCHEMA|INVALID_RESULT|INBOUND_LINKS_EXIST|RELATION_CONSTRAINT_VIOLATION|WRITE_CONFLICT|RESULT_TOO_LARGE|RECOVERY_FAILED|INTERNAL_IO_ERROR)"'
  sources="$(find crates -path 'crates/lodestar-core' -prune -o -path '*/src/*' -name '*.rs' -print)"
  matches=""
  if [[ -n "$sources" ]]; then
    matches="$(printf '%s\n' "$sources" | xargs grep -nE "$error_pattern" 2>/dev/null || true)"
    matches="$(printf '%s\n' "$matches" | grep -vE ':[[:space:]]*(//|///)' || true)"
  fi
  if [[ -n "$matches" ]]; then
    printf '%s\n' "$matches" >&2
    printf '%s\n' "ERROR: un crate fuera de lodestar-core redefine códigos de wire" >&2
    return 1
  fi

  printf '%s\n' "política de dependencias y tipos: OK"
}

gate_policy() {
  python3 scripts/check-agent-guidance.py --include-legacy
  gate_contract_static
  gate_dependency_policy
}

gate_full() {
  python3 scripts/check-agent-guidance.py --include-legacy
  gate_contract_static
  cargo fmt --all --check
  cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
  cargo build --workspace --all-targets --locked
  cargo test --workspace --locked
  cargo test -p lodestar-workspace --features test-failpoints --locked
  cargo test -p lodestar-app --features test-failpoints --locked
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
  gate_dependency_policy
  if [[ "$(uname -s)" == "Linux" ]]; then
    ./scripts/demo-smoke.sh
  else
    printf '%s\n' "demo smoke: omitido fuera de Linux (CI lo ejecuta en ubuntu-latest)"
  fi
}

case "${1:-}" in
  contract)
    gate_contract
    ;;
  policy)
    gate_policy
    ;;
  full)
    gate_full
    ;;
  *)
    printf '%s\n' "uso: scripts/agent-gates.sh {contract|policy|full}" >&2
    exit 2
    ;;
esac
