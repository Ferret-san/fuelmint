//! FuelVM as an ABCI application
use std::{
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
use futures::{executor, future::FutureExt};
use tower::Service;

use tendermint::{
    abci::{
        request::{EndBlock, Query as RequestQuery, Request},
        response, Response,
    },
    block::Height,
};

use tower_abci::BoxError;

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

use fuel_core_interfaces::common::{
    fuel_tx::{Cacheable, Transaction as FuelTx},
    fuel_vm::prelude::Deserializable,
};

// The Application
#[derive(Clone)]
pub struct App<Relayer: RelayerTrait> {
    pub committed_state: Arc<Mutex<State>>,
    pub current_state: State,
    pub relayer: Option<Relayer>,
}

impl<Relayer: RelayerTrait> Default for App<Relayer> {
    fn default() -> Self {
        Self {
            committed_state: Arc::new(Mutex::new(State::default())),
            current_state: State::default(),
            relayer: None,
        }
    }
}

impl<Relayer: RelayerTrait> App<Relayer> {
    pub fn new(state: State, relayer: Relayer) -> Self {
        let committed_state = Arc::new(Mutex::new(state.clone()));
        let current_state = state;

        App {
            committed_state,
            current_state,
            relayer: Some(relayer),
        }
    }

    pub fn new_empty(state: State) -> Self {
        let committed_state = Arc::new(Mutex::new(state.clone()));
        let current_state = state;

        App {
            committed_state,
            current_state,
            relayer: None,
        }
    }

    /// Create the header for a new block at the provided height
    async fn new_header(
        &self,
        previous_block_info: PreviousBlockInfo,
        height: BlockHeight,
    ) -> Result<PartialFuelBlockHeader> {
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
        let state = &self.current_state;

        let height = state
            .executor
            .producer
            .database
            .current_block_height()
            .unwrap();

        let is_script = tx.is_script();
        let previous_block_info = state.previous_block_info(height)?;
        let header = self.new_header(previous_block_info, height).await.unwrap();
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
        let state = &self.current_state;

        response::Info {
            data: "fuelmint".to_string(),
            version: "0.1.0".to_string(),
            app_version: 1,
            last_block_height: Height::try_from(state.block_height).unwrap(),
            last_block_app_hash: state.app_hash.to_vec().into(),
        }
    }

    // TODO: Add CheckTx for basic requirements like signatures (maybe check inputs and outputs?)
    async fn deliver_tx(&mut self, deliver_tx_request: Bytes) -> response::DeliverTx {
        tracing::trace!("delivering tx");

        let tx: Transaction = FuelTx::from_bytes(&deliver_tx_request).unwrap();
        // match serde_json::from_slice(&deliver_tx_request) {
        //     Ok(tx) => tx,
        //     Err(_) => {
        //         tracing::error!("could not decode request");
        //         return response::DeliverTx {
        //             data: "could not decode request".into(),
        //             ..Default::default()
        //         };
        //     }
        // };

        let tx_string = tx.to_json();
        // Ad tx to our list of transactions to be executed
        self.current_state.transactions.push(tx);

        tracing::trace!("tx delivered");

        response::DeliverTx {
            data: Bytes::from(tx_string),
            ..Default::default()
        }
    }

    async fn end_block(&mut self, end_block_request: EndBlock) -> response::EndBlock {
        tracing::trace!("ending block");
        // Set block height
        self.current_state.block_height = end_block_request.height;

        let height = BlockHeight::from(self.current_state.block_height as u64);

        let previous_block_info = self.current_state.previous_block_info(height).unwrap();
        // Create a partial fuel block header
        let header = self.new_header(previous_block_info, height).await.unwrap();

        // Build a block for exeuction using the header and our vec of transactions
        let block = PartialFuelBlock::new(header, self.current_state.transactions.clone());

        // Store the context string incase we error.
        let context_string = format!(
            "Failed to produce block {:?} due to execution failure",
            block
        );
        let result = self
            .current_state
            .executor
            .producer
            .execute(ExecutionBlock::Production(block))
            .context(context_string)
            .unwrap();

        tracing::trace!("done executing the block");
        tracing::debug!("Produced block with result: {:?}", &result);
        // Clear the transactions vec
        self.current_state.transactions.clear();
        // Should I make an event that returns the Execution Result?
        response::EndBlock::default()
    }

    async fn commit(&mut self) -> response::Commit {
        tracing::trace!("taking lock");
        let mut committed_state = self.committed_state.lock().await;
        *committed_state = self.current_state.clone();
        tracing::trace!("committed");

        response::Commit {
            data: Bytes::from(vec![]), // (*committed_state).app_hash.clone(),
            retain_height: 0u32.into(),
        }
    }

    async fn query(&self, query_request: RequestQuery) -> response::Query {
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
                self.current_state
                    .balance(
                        address.to_string().as_str(),
                        Some(asset_id.to_string().as_str()),
                    )
                    .unwrap(),
            ),
            Query::ContractBalance(contract_id, asset_id) => QueryResponse::ContractBalance(
                self.current_state
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

        println!("Request {:?}", req);
        let rsp = match req {
            // handled messages
            Request::Info(_) => Response::Info(executor::block_on(self.info())),
            Request::Query(query) => Response::Query(executor::block_on(self.query(query))),
            Request::DeliverTx(deliver_tx) => {
                Response::DeliverTx(executor::block_on(self.deliver_tx(deliver_tx.tx)))
            }
            Request::EndBlock(end_block) => {
                Response::EndBlock(executor::block_on(self.end_block(end_block)))
            }
            Request::Commit => Response::Commit(executor::block_on(self.commit())),
            // unhandled messages
            Request::Flush => Response::Flush,
            Request::Echo(_) => Response::Echo(Default::default()),
            Request::InitChain(_) => Response::InitChain(Default::default()),
            Request::BeginBlock(_) => Response::BeginBlock(Default::default()),
            Request::CheckTx(_) => Response::CheckTx(Default::default()),
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

        println!("Response: {:?}", rsp);
        tracing::info!(?rsp);
        async move { Ok(rsp) }.boxed()
    }
}
