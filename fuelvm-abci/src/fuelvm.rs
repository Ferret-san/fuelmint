//! FuelVM as an ABCI application
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures::future::FutureExt;
use structopt::StructOpt;
use tower::{Service, ServiceBuilder};

use tendermint::abci::{response, Event, EventAttributeIndexExt, Request, Response};

use tower_abci::{split, BoxError, Server};

use fuel_vm::{memory_client::MemoryClient, storage::MemoryStorage, transactor};

/// In-memory, hashmap-backed key-value store ABCI application.
#[derive(Clone, Debug)]
pub struct State<Client> {
    pub block_height: u32,
    pub app_hash: Vec<u8>,
    pub client: Client,
}

impl Service<Request> for State<MemoryClient> {
    type Response = Response;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Response, BoxError>> + Send + 'static>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        tracing::info!(?req);

        let rsp = match req {
            // handled messages
            Request::Info(_) => Response::Info(self.info()),
            Request::Query(query) => Response::Query(self.query(query.data)),
            Request::DeliverTx(deliver_tx) => Response::DeliverTx(self.deliver_tx(deliver_tx.tx)),
            Request::Commit => Response::Commit(self.commit()),
            // unhandled messages
            Request::Flush => Response::Flush,
            Request::Echo(_) => Response::Echo(Default::default()),
            Request::InitChain(_) => Response::InitChain(Default::default()),
            Request::BeginBlock(_) => Response::BeginBlock(Default::default()),
            Request::CheckTx(_) => Response::CheckTx(Default::default()),
            Request::EndBlock(_) => Response::EndBlock(Default::default()),
            Request::ListSnapshots => Response::ListSnapshots(Default::default()),
            Request::OfferSnapshot(_) => Response::OfferSnapshot(Default::default()),
            Request::LoadSnapshotChunk(_) => Response::LoadSnapshotChunk(Default::default()),
            Request::ApplySnapshotChunk(_) => Response::ApplySnapshotChunk(Default::default()),
        };
        tracing::info!(?rsp);
        async move { Ok(rsp) }.boxed()
    }
}

impl Default for State<MemoryClient> {
    fn default() -> Self {
        Self {
            block_height: 0,
            app_hash: Vec::new(),
            client: MemoryClient::from(MemoryStorage::default()),
        }
    }
}

// TODO: Implement all ABCI methods
impl State<MemoryClient> {
    fn info(&self) -> response::Info {
        response::Info {
            data: "fuelvm-abci".to_string(),
            version: "0.1.0".to_string(),
            app_version: 1,
            last_block_height: self.block_height.into(),
            last_block_app_hash: self.app_hash.to_vec().into(),
        }
    }

    // Paradigm replicated the eth_call interface
    // what's the Fuel equivalent?
    fn query(&self, query: Bytes) -> response::Query {
        // TODO
    }

    fn deliver_tx(&mut self, tx: Bytes) -> response::DeliverTx {
        // TODO
        // Call the `transact` method from `MemoryClient::transact`
    }

    fn commit(&mut self) -> response::Commit {
        // TODO
        // Call `MemoryClient::persist()`
    }
}

#[derive(Debug, StructOpt)]
struct Opt {
    /// Bind the TCP server to this host.
    #[structopt(short, long, default_value = "127.0.0.1")]
    host: String,

    /// Bind the TCP server to this port.
    #[structopt(short, long, default_value = "26658")]
    port: u16,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let opt = Opt::from_args();

    // Construct our ABCI application.
    let service = State::default();

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

    // Run the ABCI server.
    server
        .listen(format!("{}:{}", opt.host, opt.port))
        .await
        .unwrap();
}
