fn main() {
    if let Err(error) = lodestar_bench::run_from_args(std::env::args_os()) {
        eprintln!("lodestar-bench: {error:#}");
        std::process::exit(1);
    }
}
