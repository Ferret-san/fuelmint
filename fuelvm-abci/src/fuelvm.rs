//! FuelVM as an ABCI application
use std::{
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

use fuel_tx::{Checked, CheckedTransaction, IntoChecked, Transaction};

use fuel_core::chain_config::ChainConfig;

/// The app's state, containing a FuelVM
#[derive(Clone, Debug)]
pub struct State<Db> {
    pub block_height: i64,
    pub app_hash: Vec<u8>,
    pub db: Db,
    pub config: ChainConfig,
}

/// uses rocksdb when rocksdb features are enabled
/// uses in-memory when rocksdb features are disabled
impl Default for State<MemoryStorage> {
    fn default() -> Self {
        Self {
            block_height: 0,
            app_hash: Vec::new(),
            db: MemoryStorage::default(),
            config: ChainConfig::default(),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct TransactionResult {
    transaction: Transaction,
    gas: u64,
    result: Option<ProgramState>,
}

impl State<MemoryStorage> {
    fn into_checked_basic(&self, tx: Transaction) -> eyre::Result<CheckedTransaction> {
        let checked_tx: CheckedTransaction = tx
            .clone()
            .into_checked_basic(
                self.block_height as Word,
                &self.config.transaction_parameters,
            )?
            .into();

        Ok(checked_tx)
    }

    fn execute<Tx>(&mut self, checked_tx: Checked<Tx>) -> Result<TransactionResult, ExecutionError>
    where
        Tx: ExecutableTransaction + PartialEq,
        <Tx as IntoChecked>::Metadata: CheckedMetadata + Clone,
    {
        let tx_id = checked_tx.transaction().id();

        let mut vm = Interpreter::with_storage(&mut self.db, self.config.transaction_parameters);

        let vm_result: StateTransition<_> = vm
            .transact(checked_tx.clone())
            .map_err(|error| ExecutionError::VmExecution {
                error,
                transaction_id: tx_id,
            })?
            .into();

        if !vm_result.should_revert() {
            self.db.commit();
        }

        self.db.persist();

        Ok(TransactionResult {
            transaction: vm_result.tx().clone().into(),
            gas: vm_result.tx().price(),
            result: Some(*vm_result.state()),
        })
    }
}

// The Application
pub struct App<Db> {
    pub committed_state: Arc<Mutex<State<Db>>>,
    pub current_state: Arc<Mutex<State<Db>>>,
}

impl Default for App<MemoryStorage> {
    fn default() -> Self {
        Self {
            committed_state: Arc::new(Mutex::new(State::default())),
            current_state: Arc::new(Mutex::new(State::default())),
        }
    }
}

impl<Db: Clone> App<Db> {
    pub fn new(state: State<Db>) -> Self {
        let committed_state = Arc::new(Mutex::new(state.clone()));
        let current_state = Arc::new(Mutex::new(state));

        App {
            committed_state,
            current_state,
        }
    }
}

// TODO: Implement all ABCI methods
impl App<MemoryStorage> {
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

    // Should I worry about coinbase transactions? fees? etc?
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

        let checked_tx = state.into_checked_basic(tx).unwrap();

        let result: TransactionResult;

        match checked_tx {
            CheckedTransaction::Script(script) => result = state.execute(script).unwrap(),
            CheckedTransaction::Create(create) => result = state.execute(create).unwrap(),
            CheckedTransaction::Mint(_) => {
                // Right now, we only support `Mint` transactions for coinbase,
                // which are processed separately as a first transaction.
                //
                // All other `Mint` transaction is not allowed.
                return response::DeliverTx {
                    data: "transaction not support".into(),
                    ..Default::default()
                };
            }
        }

        tracing::trace!("executed tx");

        response::DeliverTx {
            data: Bytes::from(serde_json::to_vec(&result).unwrap()),
            ..Default::default()
        }
    }

    async fn end_block(&mut self, end_block_request: EndBlock) -> response::EndBlock {
        tracing::trace!("ending block");
        let mut current_state = self.current_state.lock().await;
        current_state.block_height = end_block_request.height;
        current_state.app_hash = vec![];
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
            data: Bytes::from((*committed_state).app_hash.clone()),
            retain_height: Height::from(0 as u32),
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

impl Service<Request> for App<MemoryStorage> {
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
