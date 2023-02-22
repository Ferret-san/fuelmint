#![allow(clippy::let_unit_value)]
use crate::{
    client::schema::build_schema, graph_api::new_service, service::SharedState, types::EmptyRelayer,
};
use fuel_core::service::adapters::P2PAdapter;
use fuel_core::{
    database::Database,
    fuel_core_graphql_api::Config as GraphQLConfig,
    producer::Producer,
    schema::dap::init,
    service::{
        adapters::{
            BlockImporterAdapter, BlockProducerAdapter, ExecutorAdapter, MaybeRelayerAdapter,
            PoAAdapter, TxPoolAdapter, VerifierAdapter,
        },
        Config, SubServices,
    },
};
use fuel_core_types::blockchain::primitives::DaBlockHeight;
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};

pub fn init_sub_services(
    config: &Config,
    database: &Database,
) -> anyhow::Result<(Box<Arc<BlockProducerAdapter>>, SubServices, SharedState)> {
    #[cfg(feature = "relayer")]
    let relayer_service = if config.relayer.eth_client.is_some() {
        Some(fuel_core_relayer::new_service(
            database.clone(),
            config.relayer.clone(),
        )?)
    } else {
        None
    };

    let relayer_adapter = MaybeRelayerAdapter {
        database: database.clone(),
        #[cfg(feature = "relayer")]
        relayer_synced: relayer_service.as_ref().map(|r| r.shared.clone()),
        #[cfg(feature = "relayer")]
        da_deploy_height: config.relayer.da_deploy_height,
    };

    let executor = ExecutorAdapter {
        relayer: relayer_adapter.clone(),
        config: config.clone(),
    };

    let verifier = VerifierAdapter::new(config, database.clone(), relayer_adapter.clone());

    let importer_adapter = BlockImporterAdapter::new(
        config.block_importer.clone(),
        database.clone(),
        executor.clone(),
        verifier.clone(),
    );

    #[cfg(not(feature = "p2p"))]
    let p2p_adapter = P2PAdapter::new();

    let txpool = fuel_core_txpool::new_service(
        config.txpool.clone(),
        database.clone(),
        importer_adapter.clone(),
        p2p_adapter.clone(),
    );
    let tx_pool_adapter = TxPoolAdapter::new(txpool.shared.clone());

    // restrict the max number of concurrent dry runs to the number of CPUs
    // as execution in the worst case will be CPU bound rather than I/O bound.
    let max_dry_run_concurrency = num_cpus::get();
    let block_producer = Producer {
        config: config.block_producer.clone(),
        db: database.clone(),
        txpool: Box::new(tx_pool_adapter.clone()),
        executor: Arc::new(executor),
        relayer: Box::new(EmptyRelayer {
            zero_height: DaBlockHeight(0),
        }),
        lock: Mutex::new(()),
        dry_run_semaphore: Semaphore::new(max_dry_run_concurrency),
    };
    let producer_adapter = BlockProducerAdapter::new(block_producer);

    // Check if we need to make the coordinator part of the services

    let poa_adapter = PoAAdapter::new(None);
    // TODO: Figure out on how to move it into `fuel-core-graphql-api`.
    let schema = init(
        build_schema(),
        config.chain_conf.transaction_parameters,
        config.chain_conf.gas_costs.clone(),
    )
    .data(database.clone());
    let gql_database = Box::new(database.clone());

    // handle schema difference
    let graph_ql = new_service(
        GraphQLConfig {
            addr: config.addr,
            utxo_validation: config.utxo_validation,
            manual_blocks_enabled: config.manual_blocks_enabled,
            vm_backtrace: config.vm.backtrace,
            min_gas_price: config.txpool.min_gas_price,
            max_tx: config.txpool.max_tx,
            max_depth: config.txpool.max_depth,
            transaction_parameters: config.chain_conf.transaction_parameters,
            consensus_key: config.consensus_key.clone(),
        },
        gql_database,
        schema,
        Box::new(producer_adapter.clone()),
        Box::new(tx_pool_adapter),
        Box::new(poa_adapter),
    )?;

    let shared = SharedState {
        txpool: txpool.shared.clone(),
        graph_ql: graph_ql.shared.clone(),
        block_importer: importer_adapter,
        config: config.clone(),
    };

    #[allow(unused_mut)]
    // `FuelService` starts and shutdowns all sub-services in the `services` order
    let mut services: SubServices = vec![
        // GraphQL should be shutdown first, so let's start it first.
        Box::new(graph_ql),
        Box::new(txpool),
    ];

    Ok((Box::new(Arc::new(producer_adapter)), services, shared))
}
