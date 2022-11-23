//! FuelVM as an ABCI application
use std::{
    fmt,
    fmt::Debug,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use tokio::sync::Mutex;

use bytes::Bytes;
use futures::future::FutureExt;
use structopt::StructOpt;
use tower::{Service, ServiceBuilder};

use tendermint::{
    abci::{
        request::{EndBlock, Query, Request},
        response, Response,
    },
    block::Height,
};

use tower_abci::{split, BoxError, Server};

use fuel_vm::{
    interpreter::CheckedMetadata,
    prelude::{ExecutableTransaction, Interpreter},
    state::ProgramState,
};

use fuel_vm::{prelude::Word, storage::MemoryStorage};

use fuel_core_interfaces::{common::state::StateTransition, executor::Error as ExecutionError};

use fuel_tx::{Bytes32, Checked, CheckedTransaction, IntoChecked, Transaction};

use fuel_core::{
    chain_config::ChainConfig,
    database::Database,
    executor::Executor as FuelExecutor,
    model::{BlockHeight, DaBlockHeight},
    schema::block::Block,
    service::Config,
};

/// Create a wrapper around fuel's Executor
pub struct Executor {
    pub producer: FuelExecutor,
}

impl Executor {
    pub fn new(producer: FuelExecutor) -> Self {
        Self { producer: producer }
    }
}

impl Debug for Executor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Executor")
            .field("database", &self.producer.database)
            .field("config", &self.producer.config)
            .finish()
    }
}

impl Clone for Executor {
    fn clone(&self) -> Executor {
        Executor {
            producer: FuelExecutor {
                database: self.producer.database.clone(),
                config: self.producer.config.clone(),
            },
        }
    }
}

impl Default for Executor {
    fn default() -> Self {
        Executor::new(FuelExecutor {
            database: Database::default(),
            config: Config::local_node(),
        })
    }
}

impl From<FuelExecutor> for Executor {
    fn from(producer: FuelExecutor) -> Self {
        Executor::new(producer)
    }
}
/// The app's state, containing a FuelVM
#[derive(Clone, Debug)]
pub struct State {
    pub block_height: i64,
    pub app_hash: Vec<u8>,
    pub transactions: Vec<Transaction>,
    pub executor: Executor,
}

impl Default for State {
    fn default() -> Self {
        Self {
            block_height: 0,
            app_hash: Vec::new(),
            transactions: Vec::new(),
            executor: Executor::default(),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct TransactionResult {
    transaction: Transaction,
    gas: u64,
    result: Option<ProgramState>,
}

struct PreviousBlockInfo {
    prev_root: Bytes32,
    da_height: DaBlockHeight,
}

impl State {
    fn previous_block_info(&self, height: BlockHeight) -> Result<PreviousBlockInfo> {
        // block 0 is reserved for genesis
        if height == 0u32.into() {
            Err(GenesisBlock.into())
        }
    }
}

// The Application
pub struct App {
    pub committed_state: Arc<Mutex<State>>,
    pub current_state: Arc<Mutex<State>>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            committed_state: Arc::new(Mutex::new(State::default())),
            current_state: Arc::new(Mutex::new(State::default())),
        }
    }
}

impl App {
    pub fn new(state: State) -> Self {
        let committed_state = Arc::new(Mutex::new(state.clone()));
        let current_state = Arc::new(Mutex::new(state));

        App {
            committed_state,
            current_state,
        }
    }
}

// TODO: Implement more ABCI methods
// CheckTx -> check for basic requirements like signatures (maybe check inputs and outputs?)
impl App {
    async fn info(&self) -> response::Info {
        let state = self.current_state.lock().await;

        response::Info {
            data: "fuelvm-abci".to_string(),
            version: "0.1.0".to_string(),
            app_version: 1,
            last_block_height: Height::try_from(state.block_height).unwrap(),
            last_block_app_hash: state.app_hash.to_vec().into(),
        }
    }

    async fn deliver_tx(&mut self, deliver_tx_request: Bytes) -> response::DeliverTx {
        tracing::trace!("delivering tx");
        let mut state = self.current_state.lock().await;

        let tx: Transaction = match serde_json::from_slice(&deliver_tx_request) {
            Ok(tx) => tx,
            Err(_) => {
                tracing::error!("could not decode request");
                return response::DeliverTx {
                    data: "could not decode request".into(),
                    ..Default::default()
                };
            }
        };

        let tx_string = tx.to_json();
        // Ad tx to our list of transactions to be executed
        state.transactions.push(tx);

        tracing::trace!("tx delivered");

        response::DeliverTx {
            data: Bytes::from(tx_string),
            ..Default::default()
        }
    }

    async fn end_block(&mut self, end_block_request: EndBlock) -> response::EndBlock {
        tracing::trace!("ending block");
        let mut current_state = self.current_state.lock().await;
        // Set block height
        current_state.block_height = end_block_request.height;
        // Create a partial fuel block header
        let header = current_state.app_hash = vec![];
        tracing::trace!("done");

        response::EndBlock::default()
    }

    async fn commit(&mut self) -> response::Commit {
        tracing::trace!("taking lock");
        let current_state = self.current_state.lock().await.clone();
        let mut committed_state = self.committed_state.lock().await;
        *committed_state = current_state;
        tracing::trace!("committed");

        response::Commit {
            data: Bytes::from(vec![]), // (*committed_state).app_hash.clone(),
            retain_height: 0u32.into(),
        }
    }

    // Paradigm replicated the eth_call interface
    // what's the Fuel equivalent?
    // async fn query(&self, query_request: Query) -> response::Query {
    //     let state = self.current_state.lock().await;

    //     // TODO: Implement a type for the available queries
    //     let query = match serde_json::from_slice(&query_request.data) {
    //         Ok(tx) => tx,
    //         // no-op just logger
    //         Err(_) => {
    //             return response::Query {
    //                 value: "could not decode request".into(),
    //                 ..Default::default()
    //             };
    //         }
    //     };

    //     // Use dry_run, return the response to the query

    //     response::Query {
    //         key: query_request.data,
    //         // value
    //         ..Default::default()
    //     }
    // }
}

impl Service<Request> for App {
    type Response = Response;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Response, BoxError>> + Send + 'static>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        tracing::info!(?req);

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();

        let rsp = match req {
            // handled messages
            Request::Info(_) => Response::Info(runtime.block_on(self.info())),
            // Need to handle query
            Request::Query(_) => Response::Query(Default::default()),
            Request::DeliverTx(deliver_tx) => {
                Response::DeliverTx(runtime.block_on(self.deliver_tx(deliver_tx.tx)))
            }
            Request::Commit => Response::Commit(runtime.block_on(self.commit())),
            // unhandled messages
            Request::Flush => Response::Flush,
            Request::Echo(_) => Response::Echo(Default::default()),
            Request::InitChain(_) => Response::InitChain(Default::default()),
            Request::BeginBlock(_) => Response::BeginBlock(Default::default()),
            Request::CheckTx(_) => Response::CheckTx(Default::default()),
            Request::EndBlock(end_block) => {
                Response::EndBlock(runtime.block_on(self.end_block(end_block)))
            }
            Request::ListSnapshots => Response::ListSnapshots(Default::default()),
            Request::OfferSnapshot(_) => Response::OfferSnapshot(Default::default()),
            Request::LoadSnapshotChunk(_) => Response::LoadSnapshotChunk(Default::default()),
            Request::ApplySnapshotChunk(_) => Response::ApplySnapshotChunk(Default::default()),
            Request::SetOption(_) => Response::SetOption(tendermint::abci::response::SetOption {
                code: 0,
                log: String::from("N/A"),
                info: String::from("N/A"),
            }),
        };
        tracing::info!(?rsp);
        async move { Ok(rsp) }.boxed()
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

    // Run the ABCI server.
    server
        .listen(format!("{}:{}", opt.host, opt.port))
        .await
        .unwrap();
}
