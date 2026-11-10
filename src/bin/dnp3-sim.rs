//! DNP3 outstation simulator: `dnp3-sim [port] [outstation address] [master address]`.

use voltrus::dnp3::outstation::{OutstationConfig, serve};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let arg = |n: usize| std::env::args().nth(n).and_then(|s| s.parse::<u16>().ok());
    let port = arg(1).unwrap_or(20000);
    let cfg = OutstationConfig {
        address: arg(2).unwrap_or(10),
        master_address: arg(3).unwrap_or(1),
        ..OutstationConfig::default()
    };
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind");
    tracing::info!(
        "DNP3 outstation simulator on {addr}, address {}, master {}",
        cfg.address,
        cfg.master_address
    );
    let _handle = serve(listener, cfg).await;
    std::future::pending::<()>().await;
}
