use lodestar_store::Store;

struct TraceEnv;

impl Drop for TraceEnv {
    fn drop(&mut self) {
        std::env::remove_var("LODESTAR_H03_SQL_TRACE");
        std::env::remove_var("LODESTAR_H03_SQL_TRACE_ROOT");
    }
}

#[test]
fn traza_sql_global_no_puede_ser_reclamada_por_otro_workspace() {
    let owner = tempfile::tempdir().expect("owner tempdir");
    let intruder = tempfile::tempdir().expect("intruder tempdir");
    std::fs::write(owner.path().join("owner.md"), "# owner\n").unwrap();
    std::fs::write(intruder.path().join("intruder.md"), "# intruder\n").unwrap();

    let trace = owner
        .path()
        .join(".lodestar/index.db.next.h03-sql-trace.ndjson");
    std::env::set_var("LODESTAR_H03_SQL_TRACE", &trace);
    let _trace_env = TraceEnv;

    Store::open_and_build(intruder.path()).expect("rebuild intruso");
    assert!(
        !trace.exists(),
        "un workspace ajeno no puede crear ni truncar la traza del propietario"
    );

    Store::open_and_build(owner.path()).expect("rebuild propietario");
    let events: Vec<serde_json::Value> = std::fs::read_to_string(&trace)
        .expect("traza del propietario")
        .lines()
        .map(|line| serde_json::from_str(line).expect("cada línea debe ser JSON completo"))
        .collect();
    assert!(
        events.len() > 2,
        "traza no vacía con header, trabajo y footer"
    );
    let build_id = events[0]["build_id"].as_str().expect("build_id del header");
    assert!(events
        .iter()
        .all(|event| { event["build_id"].as_str() == Some(build_id) }));
    assert_eq!(events.last().unwrap()["event"].as_str(), Some("footer"));
}
