use crate::tx::TxMutation;

use async_graphql::{MergedObject, MergedSubscription, Schema, SchemaBuilder};

use fuel_core::schema::{
    balance, block, chain, coin, contract, dap, health, message, node_info, resource, tx,
};

// re-use fuel's Query object for the schema
#[derive(MergedObject, Default)]
pub struct Query(
    dap::DapQuery,
    balance::BalanceQuery,
    block::BlockQuery,
    chain::ChainQuery,
    tx::TxQuery,
    health::HealthQuery,
    coin::CoinQuery,
    contract::ContractQuery,
    contract::ContractBalanceQuery,
    node_info::NodeQuery,
    message::MessageQuery,
    resource::ResourceQuery,
);

// Add the new TxMutation
#[derive(MergedObject, Default)]
pub struct Mutation(dap::DapMutation, TxMutation, block::BlockMutation);

// re-use Subscription
#[derive(MergedSubscription, Default)]
pub struct Subscription(tx::TxStatusSubscription);

// Build the CoreSchema
pub type CoreSchema = Schema<Query, Mutation, Subscription>;
pub type CoreSchemaBuilder = SchemaBuilder<Query, Mutation, Subscription>;

pub fn build_schema() -> SchemaBuilder<Query, Mutation, Subscription> {
    Schema::build_with_ignore_name_conflicts(
        Query::default(),
        Mutation::default(),
        Subscription::default(),
        ["TransactionConnection", "MessageConnection"],
    )
}
