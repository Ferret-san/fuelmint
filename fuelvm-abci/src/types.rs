//! FuelVM as an ABCI application
use std::{
    fmt::Debug,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use crate::state::*;

use crate::queries::*;

use anyhow::{anyhow, Context as ContextTrait, Result};

use tokio::sync::Mutex;

use bytes::Bytes;
use futures::future::FutureExt;
use structopt::StructOpt;
use tower::{Service, ServiceBuilder};

use tendermint::{
    abci::{
        request::{EndBlock, Query as RequestQuery, Request},
        response, Response,
    },
    block::Height,
};

use tower_abci::{split, BoxError, Server};

//use fuel_core::schema::balance::{Balance, BalanceQuery};

use fuel_core_interfaces::{
    block_producer::{Error::InvalidDaFinalizationState, Relayer as RelayerTrait},
    common::tai64::Tai64,
    executor::{ExecutionBlock, Executor as ExecutorTrait},
    model::{
        BlockHeight, DaBlockHeight, FuelApplicationHeader, FuelConsensusHeader, PartialFuelBlock,
        PartialFuelBlockHeader,
    },
};

use fuel_tx::{Receipt, Transaction};

use fuel_block_producer::db::BlockProducerDatabase;

/// TODO
/// 1. Connect the FuelClient to my app for GraphQL queries and mutations
/// 2. Make usre I initialize a database and what not
/// 3. Initialize the Fuel Service
/// 4. Connect the Client with the service

// The Application
pub struct App<Relayer: RelayerTrait> {
    pub committed_state: Arc<Mutex<State>>,
    pub current_state: Arc<Mutex<State>>,
    pub relayer: Option<Relayer>,
}

impl<Relayer: RelayerTrait> Default for App<Relayer> {
    fn default() -> Self {
        Self {
            committed_state: Arc::new(Mutex::new(State::default())),
            current_state: Arc::new(Mutex::new(State::default())),
            relayer: None,
        }
    }
}

impl<Relayer: RelayerTrait> App<Relayer> {
    pub fn new(state: State, relayer: Relayer) -> Self {
        let committed_state = Arc::new(Mutex::new(state.clone()));
        let current_state = Arc::new(Mutex::new(state));

        App {
            committed_state,
            current_state,
            relayer: Some(relayer),
        }
    }

    /// Create the header for a new block at the provided height
    async fn new_header(&self, height: BlockHeight) -> Result<PartialFuelBlockHeader> {
        let state = self.current_state.lock().await;
        let previous_block_info = state.previous_block_info(height)?;
        let new_da_height = self
            .select_new_da_height(previous_block_info.da_height)
            .await?;

        Ok(PartialFuelBlockHeader {
            application: FuelApplicationHeader {
                da_height: new_da_height,
                generated: Default::default(),
            },
            consensus: FuelConsensusHeader {
                // TODO: this needs to be updated using a proper BMT MMR
                prev_root: previous_block_info.prev_root,
                height,
                time: Tai64::now(),
                generated: Default::default(),
            },
            metadata: None,
        })
    }

    async fn select_new_da_height(
        &self,
        previous_da_height: DaBlockHeight,
    ) -> Result<DaBlockHeight> {
        match &self.relayer {
            Some(relayer) => {
                let best_height = relayer.get_best_finalized_da_height().await?;
                if best_height < previous_da_height {
                    // If this happens, it could mean a block was erroneously imported
                    // without waiting for our relayer's da_height to catch up to imported da_height.
                    return Err(InvalidDaFinalizationState {
                        best: best_height,
                        previous_block: previous_da_height,
                    }
                    .into());
                }
                Ok(best_height)
            }
            // If we do not have a relayer, we can just return zero
            // Meaning we dont process deposits/withdrawals or messages from some settlement layer / L1
            None => Ok(DaBlockHeight::from(0u64)),
        }
    }

    async fn dry_run(&self, tx: Transaction) -> Result<Vec<Receipt>> {
        let state = self.current_state.lock().await;

        let height = state
            .executor
            .producer
            .database
            .current_block_height()
            .unwrap();

        let is_script = tx.is_script();
        let header = self.new_header(height).await.unwrap();
        let block = PartialFuelBlock::new(header, vec![tx].into_iter().collect());

        let result: Vec<_> = state
            .executor
            .producer
            .dry_run(
                ExecutionBlock::Production(block),
                Some(state.executor.producer.config.utxo_validation),
            )
            .unwrap()
            .into_iter()
            .flatten()
            .collect();

        if is_script && result.is_empty() {
            return Err(anyhow!("Expected at least one set of receipts"));
        }
        Ok(result)
    }
}

impl<Relayer: RelayerTrait> App<Relayer> {
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

    // TODO: Add CheckTx for basic requirements like signatures (maybe check inputs and outputs?)

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
        let header = self
            .new_header(BlockHeight::from(current_state.block_height as u64))
            .await
            .unwrap();

        // Build a block for exeuction using the header and our vec of transactions
        let block = PartialFuelBlock::new(header, current_state.transactions.clone());

        // Store the context string incase we error.
        let context_string = format!(
            "Failed to produce block {:?} due to execution failure",
            block
        );
        let result = current_state
            .executor
            .producer
            .execute(ExecutionBlock::Production(block))
            .context(context_string)
            .unwrap();

        tracing::trace!("done executing the block");
        tracing::debug!("Produced block with result: {:?}", &result);
        // Clear the transactions vec
        current_state.transactions.clear();
        // Should I make an event that returns the Execution Result?
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

    async fn query(&self, query_request: RequestQuery) -> response::Query {
        let state = self.current_state.lock().await;

        let query: Query = match serde_json::from_slice(&query_request.data) {
            Ok(tx) => tx,
            // no-op just logger
            Err(_) => {
                return response::Query {
                    value: "could not decode request".into(),
                    ..Default::default()
                };
            }
        };

        let res = match query {
            Query::DryRun(tx) => {
                let result = self.dry_run(tx).await.unwrap();

                QueryResponse::Receipts(result)
            }
            Query::Balance(address, asset_id) => QueryResponse::Balance(
                state
                    .balance(
                        address.to_string().as_str(),
                        Some(asset_id.to_string().as_str()),
                    )
                    .unwrap(),
            ),
            Query::ContractBalance(contract_id, asset_id) => QueryResponse::ContractBalance(
                state
                    .contract_balance(
                        contract_id.to_string().as_str(),
                        Some(asset_id.to_string().as_str()),
                    )
                    .unwrap(),
            ),
        };

        response::Query {
            key: query_request.data,
            value: Bytes::from(serde_json::to_vec(&res).unwrap()),
            ..Default::default()
        }
    }
}

#[derive(Default, Clone)]
pub struct EmptyRelayer {
    pub zero_height: DaBlockHeight,
}

#[async_trait::async_trait]
impl RelayerTrait for EmptyRelayer {
    async fn get_best_finalized_da_height(&self) -> Result<DaBlockHeight> {
        Ok(self.zero_height)
    }
}

impl Service<Request> for App<EmptyRelayer> {
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
            Request::Query(query) => Response::Query(runtime.block_on(self.query(query))),
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