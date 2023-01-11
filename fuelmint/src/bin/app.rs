use fuelvm_abci::{graph_api::start_server, types::App};
use structopt::StructOpt;
use tokio::{sync::oneshot, task::JoinHandle};
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
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let opt = Opt::from_args();

    // Construct our ABCI application.
    let service = App::default();

    // Split it into components.
    let (consensus, mempool, snapshot, info) = split::service(service, 1);

    // Hand those components to the ABCI server, but customize request behavior
    // for each category -- for instance, apply load-shedding only to mempool
    // and info requests, but not to consensus requests.
    let server = Server::builder()
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
        .unwrap();

    println!("Litening on {}::{}", opt.host, opt.port);
    // Run the ABCI server.
    server
        .listen(format!("{}:{}", opt.host, opt.port))
        .await
        .unwrap();

    // Use persistent storage to start the graphql service
    // let (stop_tx, stop_rx) = oneshot::channel::<()>();

    // let (bound_address, api_server) = start_server(
    //     service.current_state.executor.producer.config.clone(),
    //     service.current_state.executor.producer.database,
    //     stop_rx,
    // )
    // .await
    // .unwrap();
}
