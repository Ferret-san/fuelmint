use crate::tx::TxMutation;

use async_graphql::{MergedObject, MergedSubscription, Schema, SchemaBuilder};

use fuel_core::schema::{block, dap, tx, Query};

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
