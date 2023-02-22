//! FuelVM as an ABCI application
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use crate::state::*;

use anyhow::Result;

use fuel_core::service::Config;

use itertools::Itertools;
use tokio::sync::Mutex;

use bytes::Bytes;
use futures::{executor, future::FutureExt};
use tower::Service;

use tendermint::{
    abci::{
        request::{EndBlock, Request},
        response, Response,
    },
    block::Height,
};

use tower_abci::BoxError;

use fuel_core_types::{
    fuel_tx::{Cacheable, Transaction as FuelTx, Transaction, UniqueIdentifier},
    fuel_types::bytes::Deserializable,
};

use fuel_core::{
    database::Database,
    producer::ports::Relayer as RelayerTrait,
    service::adapters::{BlockProducerAdapter, P2PAdapter},
    types::blockchain::primitives::{BlockHeight, DaBlockHeight},
};

pub struct App {
    pub config: Config,
    pub committed_state: Arc<Mutex<State>>,
    pub current_state: Arc<Mutex<State>>,
    pub producer: Box<Arc<BlockProducerAdapter>>,
    pub tx_pool: Box<fuel_core_txpool::service::SharedState<P2PAdapter, Database>>,
}

impl App {
    pub fn new(
        config: Config,
        state: State,
        producer: Box<Arc<BlockProducerAdapter>>,
        tx_pool: Box<fuel_core_txpool::service::SharedState<P2PAdapter, Database>>,
    ) -> Self {
        let committed_state = Arc::new(Mutex::new(state.clone()));
        let current_state = Arc::new(Mutex::new(state));
        App {
            config,
            committed_state,
            current_state,
            producer: producer,
            tx_pool: tx_pool,
        }
    }

    async fn info(&self) -> response::Info {
        let state = self.current_state.lock().await;

        response::Info {
            data: "fuelmint".to_string(),
            version: "0.1.0".to_string(),
            app_version: 1,
            last_block_height: Height::try_from(state.block_height).unwrap(),
            last_block_app_hash: state.app_hash.to_vec().into(),
        }
    }

    async fn deliver_tx(&mut self, deliver_tx_request: Bytes) -> response::DeliverTx {
        let tx_pool = self.tx_pool.clone();

        let mut tx: Transaction = FuelTx::from_bytes(&deliver_tx_request).unwrap();

        // Add transaction to the TxPool
        tx.precompute();
        let _: Vec<_> = tx_pool
            .insert(vec![Arc::new(tx.clone())])
            .into_iter()
            .try_collect()
            .unwrap();

        tracing::trace!("tx delivered");

        response::DeliverTx {
            data: Bytes::from(tx.to_json()),
            ..Default::default()
        }
    }

    async fn end_block(&mut self, end_block_request: EndBlock) -> response::EndBlock {
        tracing::info!("ending block");
        let mut current_state = self.current_state.lock().await;
        // Set block height
        current_state.block_height = end_block_request.height;

        let height = BlockHeight::from(current_state.block_height as u64);

        tracing::info!("Producing new block...");

        // previous_block_info breaks here at height 1, consider going back to using Executor with custom
        // function, or make a modified branch from fuel-core main to address this issue
        // Is there a need for rollup blocks to be sealed?
        // Look into adding block_time to produce_and_execute_block
        let (result, db_transaction) = self
            .producer
            .block_producer
            .produce_and_execute_block(height, None, self.config.chain_conf.block_gas_limit)
            .await
            .unwrap()
            .into();

        // commit the changes
        db_transaction.commit().unwrap();

        tracing::info!(
            "New block produced for height: {:?}",
            current_state.block_height
        );

        // Remove transactions from the txpool
        let mut tx_ids_to_remove = Vec::with_capacity(result.skipped_transactions.len());
        for (tx, err) in result.skipped_transactions {
            tracing::error!(
                "During block production got invalid transaction {:?} with error {:?}",
                tx,
                err
            );
            tx_ids_to_remove.push(tx.id());
        }
        self.tx_pool.remove_txs(tx_ids_to_remove);

        response::EndBlock::default()
    }

    async fn commit(&mut self) -> response::Commit {
        let current_state = self.current_state.lock().await.clone();
        let mut committed_state = self.committed_state.lock().await;
        *committed_state = current_state;

        response::Commit {
            data: Bytes::from(vec![]), // (*committed_state).app_hash.clone(),
            retain_height: 0u32.into(),
        }
    }
}

#[derive(Default, Clone)]
pub struct EmptyRelayer {
    pub zero_height: DaBlockHeight,
}

#[async_trait::async_trait]
impl RelayerTrait for EmptyRelayer {
    async fn wait_for_at_least(&self, _height: &DaBlockHeight) -> anyhow::Result<DaBlockHeight> {
        Ok(self.zero_height)
    }
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
        let rsp = match req {
            // handled messages
            Request::Info(_) => Response::Info(executor::block_on(self.info())),
            Request::DeliverTx(deliver_tx) => {
                Response::DeliverTx(executor::block_on(self.deliver_tx(deliver_tx.tx)))
            }
            Request::EndBlock(end_block) => {
                Response::EndBlock(executor::block_on(self.end_block(end_block)))
            }
            Request::Commit => Response::Commit(executor::block_on(self.commit())),
            // unhandled messages
            Request::Query(_) => Response::Query(Default::default()),
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

        tracing::info!(?rsp);
        async move { Ok(rsp) }.boxed()
    }
}
