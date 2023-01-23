use fuel_core::{
    database::Database,
    service::config::{Config, DbType},
};
use fuel_core_services::Service;
use fuelmint::{service, state::State, types::App};
use std::{
    net::{Ipv4Addr, SocketAddr},
    panic,
};
use structopt::StructOpt;
use tower::ServiceBuilder;
use tower_abci::{split, Server};

#[derive(Debug, StructOpt)]
struct Opt {
    /// Bind the TCP server to this host.
    #[structopt(short, long, default_value = "127.0.0.1")]
    host: String,

    /// Bind the TCP server to this port.
    #[structopt(short, long, default_value = "26658")]
    port: u16,
}

// TODO
// Add a way to define Config through a cli param
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let opt = Opt::from_args();

    let mut config = Config::local_node();
    config.addr = SocketAddr::new(Ipv4Addr::new(127, 0, 0, 1).into(), 4000);
    config.database_path = dirs::home_dir().unwrap().join(".fuel").join("db");
    config.database_type = DbType::RocksDb;
    // This is already false by default, but its left here as a reminder
    // to enable more changes to the configuration
    config.utxo_validation = false;

    tracing::info!("Database path {:?}", config.database_path);
    let database = Database::open(&config.database_path).unwrap();
    // instantiate the FuelmintService
    let (block_producer, server) = service::FuelmintService::new(database, config.clone()).unwrap();
    let txpool = Box::new(server.shared.txpool.clone());

    // currently the stop signal is being sent immediatly, which shouldn't happen

    let state = State {
        block_height: 0,
        app_hash: Vec::new(),
    };

    // Construct our ABCI application.
    let service = App::new(config.clone(), state, block_producer, txpool);

    // Split it into components.
    let (consensus, mempool, snapshot, info) = split::service(service, 1);

    // Hand those components to the ABCI server, but customize request behavior
    // for each category -- for instance, apply load-shedding only to mempool
    // and info requests, but not to consensus requests.
    // Spawn a task to run the ABCI server
    let abci_server = tokio::task::spawn(
        Server::builder()
            .consensus(consensus)
            .snapshot(snapshot)
            .mempool(
                ServiceBuilder::new()
                    .load_shed()
                    .buffer(10)
                    .service(mempool),
            )
            .info(
                ServiceBuilder::new()
                    .load_shed()
                    .buffer(100)
                    .rate_limit(50, std::time::Duration::from_secs(1))
                    .service(info),
            )
            .finish()
            .unwrap()
            .listen(format!("{}:{}", opt.host, opt.port)),
    );

    tracing::info!("ABCI server listening on {}::{}", opt.host, opt.port);

    let fuelmint_services = tokio::task::spawn(async move {
        server.start_and_await().await.unwrap();
        server.await_stop().await.unwrap();
    });

    tokio::select! {
        x = abci_server => x.unwrap().map_err(|e| anyhow::anyhow!(e)).unwrap(),
        x = fuelmint_services => x.unwrap()
    };
}
