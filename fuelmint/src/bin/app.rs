use fuel_core::{
    database::Database,
    executor::Executor as FuelExecutor,
    service::config::{Config, DbType},
};
use fuelvm_abci::{executor::Executor, graph_api::start_server, state::State, types::App};
use std::{
    net::{Ipv4Addr, SocketAddr},
    panic,
};
use structopt::StructOpt;
use tokio::sync::oneshot;
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
// Add function to initialize state from genesis if there's any
// Use RocksDB and instantiate ABCI server app + RPC
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

    println!("Database path {:?}", config.database_path);
    let database = Database::open(&config.database_path).unwrap();

    // Build executor
    let fuel_executor = FuelExecutor {
        database: database.clone(),
        config: config.clone(),
    };
    let executor = Executor::new(fuel_executor);
    // TODO: Get DB type from Config thorugh cli
    let state = State {
        block_height: 0,
        app_hash: Vec::new(),
        transactions: Vec::new(),
        executor,
    };

    // Construct our ABCI application.
    let service = App::new(state, None);

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

    println!("ABCI server listening on {}::{}", opt.host, opt.port);

    let (_stop_graphql_api, stop_graphql_rx) = oneshot::channel::<()>();

    let (bound_address, _api_server) = start_server(config.clone(), database, stop_graphql_rx)
        .await
        .unwrap();

    // let (_shutdown, stop_rx) = oneshot::channel::<()>();

    println!(
        "Graphql server listening on {}::{}",
        bound_address.ip(),
        bound_address.port()
    );

    tokio::select! {
        x = abci_server => x.unwrap().map_err(|e| anyhow::anyhow!(e)).unwrap()
    };
}
