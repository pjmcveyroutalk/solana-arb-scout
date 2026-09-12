fn main() {
    if rustls::crypto::CryptoProvider::get_default().is_none()
        && rustls::crypto::ring::default_provider()
            .install_default()
            .is_err()
    {
        eprintln!("scout_local_rpc_fatal: failed to install rustls crypto provider");
        std::process::exit(1);
    }

    if let Err(error) = scout_cli::local_rpc::run() {
        eprintln!("scout_local_rpc_fatal: {error}");
        std::process::exit(1);
    }
}
