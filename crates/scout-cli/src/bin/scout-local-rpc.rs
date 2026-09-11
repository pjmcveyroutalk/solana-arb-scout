fn main() {
    if let Err(error) = scout_cli::local_rpc::run() {
        eprintln!("scout_local_rpc_fatal: {error}");
        std::process::exit(1);
    }
}
