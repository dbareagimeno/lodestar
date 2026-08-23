//! E33-H02/L12 — regresiones del contrato de protocolo y del arnés `raw_line`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

static RAW_LINE_SELFTEST: Mutex<()> = Mutex::new(());

#[test]
fn l12_rob_15_entradas_sin_id_exigen_silencio_y_sesion_viva() {
    run_selftest("l12_batch_expect_selftest.py");
}

fn run_selftest(name: &str) {
    let _serial = lock_python_selftest();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(name);
    assert!(script.is_file(), "selftest ausente: {}", script.display());
    let output = Command::new("python3")
        .arg(&script)
        .output()
        .unwrap_or_else(|error| panic!("python3 no pudo ejecutar {}: {error}", script.display()));
    assert!(
        output.status.success(),
        "selftest {name} detectó el defecto; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_raw_line_selftest(selector: &str) {
    // Cada selector mide plazos de decenas de milisegundos. Cargo ejecuta estos 69 tests en
    // paralelo por defecto, y la contención entre sus procesos Python falsea la medición en
    // runners pequeños (especialmente macOS). La exclusión sólo afecta a esta sonda temporal.
    let _serial = lock_python_selftest();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/raw_line_selftest.py");
    assert!(script.is_file(), "selftest ausente: {}", script.display());

    let mut child = Command::new("python3")
        .arg(&script)
        .arg(selector)
        .env("PYTHONUNBUFFERED", "1")
        .spawn()
        .expect("python3 debe poder ejecutar el selftest");
    // Es un watchdog contra cuelgues, no el presupuesto funcional de cada operación.
    // Algunos selectores encadenan decenas de casos y macOS CI puede superar 3 s.
    let limite = Duration::from_secs(10);
    let inicio = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("consultar estado del selftest") {
            assert!(inicio.elapsed() < limite, "selftest terminó fuera de plazo");
            assert!(
                status.success(),
                "selftest raw_line {selector} detectó el defecto"
            );
            break;
        }
        if inicio.elapsed() >= limite {
            let _ = child.kill();
            let _ = child.wait();
            panic!("selftest raw_line excedió el límite de {limite:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn lock_python_selftest() -> std::sync::MutexGuard<'static, ()> {
    RAW_LINE_SELFTEST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn l12_raw_line_json_object_without_jsonrpc_returns_invalid_request() {
    run_raw_line_selftest("invalid-request");
}

#[test]
fn l12_raw_line_injected_rejected_id_domain_returns_observed_invalid_request_without_id() {
    run_raw_line_selftest("injected-id-domain");
}

#[test]
fn l12_raw_line_fresh_valid_id_with_non_object_params_returns_observed_idless_error() {
    run_raw_line_selftest("fresh-idless-invalid-params");
}

#[test]
fn l12_raw_line_integer_id_does_not_correlate_bool_float_or_string_aliases() {
    run_raw_line_selftest("strict-integer-id");
}

#[test]
fn l12_raw_line_params_null_is_absent_and_correlates_the_integer_id() {
    run_raw_line_selftest("null-params-correlates");
}

#[test]
fn l12_read_response_integer_id_discards_bool_float_and_string_aliases() {
    run_raw_line_selftest("strict-read-response-id");
}

#[test]
fn l12_raw_line_non_string_method_returns_observed_invalid_request_instead_of_silence() {
    run_raw_line_selftest("non-string-method-observed-frame");
}

#[test]
fn l12_raw_line_malformed_json_returns_observed_parse_error_instead_of_fabricated_silence() {
    run_raw_line_selftest("malformed-observed-frame");
}

#[test]
fn l12_raw_line_notification_returns_observed_method_error_instead_of_fabricated_silence() {
    run_raw_line_selftest("notification-observed-frame");
}

#[test]
fn l12_raw_line_recovers_buffered_id_2_after_expired_id_1() {
    run_raw_line_selftest("prefetched");
}

#[test]
fn l12_raw_line_discards_late_idless_error_before_current_id_2_response() {
    run_raw_line_selftest("late-idless-error");
}

#[test]
fn l12_raw_line_silent_input_discards_late_expired_request_response() {
    run_raw_line_selftest("silent-discards-expired-id");
}

#[test]
fn l12_raw_line_silent_input_discards_late_idless_error_from_expired_invalid_input() {
    run_raw_line_selftest("silent-discards-expired-idless-error");
}

#[test]
fn l12_raw_line_rejected_bool_id_does_not_consume_next_fresh_idless_error() {
    run_raw_line_selftest("rejected-bool-does-not-consume-fresh-idless");
}

#[test]
fn l12_raw_line_silent_input_returns_fresh_valid_id_without_expired_request() {
    run_raw_line_selftest("silence-returns-fresh-valid-id");
}

#[test]
fn l12_raw_line_silent_input_preserves_string_alias_of_expired_integer_id() {
    run_raw_line_selftest("silence-preserves-expired-id-type-alias");
}

#[test]
fn l12_raw_line_second_invalid_observes_its_fresh_idless_error_after_silent_invalid() {
    run_raw_line_selftest("second-invalid-keeps-fresh-idless");
}

#[test]
fn l12_raw_line_notification_observes_fresh_id_null_frame_after_silent_invalid() {
    run_raw_line_selftest("notification-keeps-fresh-id-null");
}

#[test]
fn l12_rpc_does_not_reuse_expired_raw_id_or_accept_stale_raw_response() {
    run_raw_line_selftest("rpc-avoids-expired-raw-id");
}

#[test]
fn l12_barrier_preserves_fresh_frame_and_avoids_pending_string_id_collision() {
    run_raw_line_selftest("barrier-preserves-fresh-and-avoids-string-id");
}

#[test]
fn l12_raw_request_preserves_foreign_frame_for_later_raw_in_fifo_order() {
    run_raw_line_selftest("raw-request-preserves-foreign-fifo");
}

#[test]
fn l12_rpc_read_response_preserves_foreign_frame_for_later_raw_in_fifo_order() {
    run_raw_line_selftest("rpc-preserves-foreign-fifo");
}

#[test]
fn l12_barrier_preserves_fresh_id_null_error_for_next_public_raw() {
    run_raw_line_selftest("barrier-preserves-fresh-id-null");
}

#[test]
fn l12_barrier_discards_at_most_one_attributable_idless_32600() {
    run_raw_line_selftest("barrier-discards-at-most-one-32600");
}

#[test]
fn l12_pending_id_does_not_consume_server_request_with_same_id() {
    run_raw_line_selftest("pending-id-preserves-server-request");
}

#[test]
fn l12_pending_id_consumption_requires_strict_jsonrpc_response_shape() {
    run_raw_line_selftest("pending-id-strict-response-shape");
}

#[test]
fn l12_reused_pending_raw_id_barrier_drains_stale_before_fresh_second() {
    run_raw_line_selftest("reused-raw-id-drains-stale");
}

#[test]
fn l12_reused_pending_raw_id_barrier_ack_extinguishes_unresolved_debt() {
    run_raw_line_selftest("reused-raw-id-ack-clears-debt");
}

#[test]
fn l12_noncolliding_raw_id_does_not_barrier_other_pending_id() {
    run_raw_line_selftest("noncolliding-raw-id-no-barrier");
}

#[test]
fn l12_pending_id_preserves_error_null_until_real_valid_response() {
    run_raw_line_selftest("pending-id-preserves-error-null");
}

#[test]
fn l12_pending_id_validates_error_object_and_accepts_any_result_json() {
    run_raw_line_selftest("pending-id-validates-error-result");
}

#[test]
fn l12_rpc_preserves_nonfinite_response_frames_as_unparseable_fifo() {
    run_raw_line_selftest("rpc-preserves-nonfinite-as-unparseable");
}

#[test]
fn l12_raw_line_strict_json_rejects_nonfinite_responses_and_inputs() {
    run_raw_line_selftest("raw-line-strict-json-nonfinite");
}

#[test]
fn l12_current_invalid_preserves_fresh_idless_before_attributable_32600_fifo() {
    run_raw_line_selftest("current-invalid-preserves-fresh-idless-fifo");
}

#[test]
fn l12_current_invalid_correlates_only_strict_idless_32600_error() {
    run_raw_line_selftest("current-invalid-strict-idless-32600");
}

#[test]
fn l12_absent_and_null_id_invalid_params_are_equivalent_invalid_requests() {
    run_raw_line_selftest("idless-null-absent-invalid-params");
}

#[test]
fn l12_resync_timeout_or_eof_raises_before_raw_or_rpc_public_send() {
    run_raw_line_selftest("resync-failure-aborts-public-send");
}

#[test]
fn l12_raw_line_resync_write_errors_are_runtime_errors_with_cause() {
    run_raw_line_selftest("raw-resync-write-errors");
}

#[test]
fn l12_rpc_resync_write_errors_are_runtime_errors_with_cause() {
    run_raw_line_selftest("rpc-resync-write-errors");
}

#[test]
fn l12_raw_line_partial_barrier_flush_failure_clears_both_barriers_and_sync_state() {
    run_raw_line_selftest("raw-partial-barrier-flush");
}

#[test]
fn l12_rpc_partial_barrier_flush_failure_clears_both_barriers_and_sync_state() {
    run_raw_line_selftest("rpc-partial-barrier-flush");
}

#[test]
fn l12_raw_line_does_not_depend_on_process_global_signal_apis() {
    run_raw_line_selftest("portable");
}

#[test]
fn l12_raw_line_timeout_50ms_returns_before_blocked_read_without_timer_delivery() {
    run_raw_line_selftest("bounded-timeout");
}

#[test]
fn l12_read_response_eof_reports_id_stderr_and_exit_without_waiting_for_timeout() {
    run_raw_line_selftest("read-response-eof");
}

#[test]
fn l12_raw_line_eof_reports_server_exit_instead_of_fabricating_silence() {
    run_raw_line_selftest("raw-line-eof");
}

#[test]
fn l12_raw_line_and_resync_persist_live_transport_eof_across_repeated_calls() {
    run_raw_line_selftest("persistent-live-eof-raw-resync");
}

#[test]
fn l12_rpc_persists_live_transport_eof_across_repeated_calls() {
    run_raw_line_selftest("persistent-live-eof-rpc");
}

#[test]
fn l12_raw_line_terminal_eof_preflight_is_stable_and_never_rewrites() {
    run_raw_line_selftest("terminal-eof-raw-preflight");
}

#[test]
fn l12_rpc_terminal_eof_preflight_caches_diagnostic_and_never_rewrites() {
    run_raw_line_selftest("terminal-eof-rpc-preflight");
}

#[test]
fn l12_raw_line_drains_queued_response_before_observing_terminal_exit() {
    run_raw_line_selftest("queued-response-before-terminal-raw");
}

#[test]
fn l12_rpc_drains_queued_response_before_observing_terminal_exit() {
    run_raw_line_selftest("queued-response-before-terminal-rpc");
}

#[test]
fn l12_raw_line_preserves_foreign_after_correlated_before_terminal() {
    run_raw_line_selftest("correlated-then-foreign-before-terminal-raw");
}

#[test]
fn l12_rpc_preserves_foreign_after_correlated_before_terminal() {
    run_raw_line_selftest("correlated-then-foreign-before-terminal-rpc");
}

#[test]
fn l12_raw_line_live_eof_returns_preserved_foreign_before_terminal() {
    run_raw_line_selftest("live-eof-preserves-foreign-raw");
}

#[test]
fn l12_rpc_then_raw_live_eof_returns_preserved_foreign_before_terminal() {
    run_raw_line_selftest("live-eof-preserves-foreign-rpc");
}

#[test]
fn l12_raw_line_terminal_drain_preserves_three_pre_eof_frames_fifo() {
    run_raw_line_selftest("terminal-drains-three-pre-eof-raw");
}

#[test]
fn l12_rpc_terminal_drain_preserves_three_pre_eof_frames_fifo() {
    run_raw_line_selftest("terminal-drains-three-pre-eof-rpc");
}

#[test]
fn l12_raw_line_resync_drains_queued_ack_before_terminal_public_preflight() {
    run_raw_line_selftest("queued-barrier-ack-before-terminal-raw");
}

#[test]
fn l12_rpc_resync_drains_queued_ack_before_terminal_public_preflight() {
    run_raw_line_selftest("queued-barrier-ack-before-terminal-rpc");
}

#[test]
fn l12_raw_line_acknowledged_barrier_beats_flush_error_and_terminal() {
    run_raw_line_selftest("ack-before-flush-error-raw");
}

#[test]
fn l12_rpc_acknowledged_barrier_beats_flush_error_and_terminal() {
    run_raw_line_selftest("ack-before-flush-error-rpc");
}

#[test]
fn l12_raw_line_past_deadline_still_drains_ack_after_flush_error() {
    run_raw_line_selftest("past-deadline-ack-after-flush-raw");
}

#[test]
fn l12_rpc_past_deadline_still_drains_ack_after_flush_error() {
    run_raw_line_selftest("past-deadline-ack-after-flush-rpc");
}

#[test]
fn l12_raw_line_past_deadline_foreign_stops_before_queued_ack() {
    run_raw_line_selftest("past-deadline-foreign-before-ack-raw");
}

#[test]
fn l12_rpc_past_deadline_foreign_stops_before_queued_ack() {
    run_raw_line_selftest("past-deadline-foreign-before-ack-rpc");
}

#[test]
fn l12_raw_line_post_deadline_probe_calls_stdout_item_with_zero_timeout() {
    run_raw_line_selftest("post-deadline-probe-zero-timeout-raw");
}

#[test]
fn l12_rpc_post_deadline_probe_calls_stdout_item_with_zero_timeout() {
    run_raw_line_selftest("post-deadline-probe-zero-timeout-rpc");
}

#[test]
fn l12_close_terminates_real_process_and_stdout_reader_within_a_bounded_wait() {
    run_raw_line_selftest("close-lifecycle");
}
