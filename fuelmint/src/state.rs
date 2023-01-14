use std::fmt::Debug;

use crate::executor::Executor;

use anyhow::Result;

use fuel_core_interfaces::{
    block_producer::Error::{GenesisBlock, MissingBlock},
    common::crypto::ephemeral_merkle_root,
    common::fuel_storage::StorageAsRef,
    common::prelude::{Address, AssetId, ContractId},
    db::ContractsAssets,
    model::{BlockHeight, DaBlockHeight},
};

use fuel_tx::{Bytes32, Transaction};

use fuel_core::{
    database::resource::{AssetQuery, AssetSpendTarget},
    state::Error,
};

use fuel_block_producer::db::BlockProducerDatabase;

/// The app's state, containing a FuelVM
#[derive(Clone, Debug)]
pub struct State {
    pub block_height: i64,
    pub app_hash: Vec<u8>,
    // Might make sense to run a fuel_tx_pool instead?
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

pub struct PreviousBlockInfo {
    pub prev_root: Bytes32,
    pub da_height: DaBlockHeight,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Balance {
    owner: Address,
    amount: u64,
    asset_id: AssetId,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct ContractBalance {
    contract: ContractId,
    amount: u64,
    asset_id: AssetId,
}

// Note: consider implementing `balances` and `contract_balances`
impl State {
    pub fn balance(&self, owner: &str, asset_id: Option<&str>) -> Result<u64> {
        let db = &self.executor.producer.database;
        let owner = owner.parse().unwrap();
        let asset_id = match asset_id {
            Some(asset_id) => asset_id.parse().unwrap(),
            None => AssetId::default(),
        };

        let balance = AssetQuery::new(
            &owner,
            &AssetSpendTarget::new(asset_id, u64::MAX, u64::MAX),
            None,
            db,
        )
        .unspent_resources()
        .map(|res| res.map(|resource| *resource.amount()))
        .try_fold(
            Balance {
                owner,
                amount: 0u64,
                asset_id,
            },
            |mut balance, res| -> Result<_, Error> {
                let amount = res?;

                // Increase the balance
                balance.amount += amount;

                Ok(balance)
            },
        )
        .unwrap();

        Ok(balance.amount)
    }

    pub fn contract_balance(&self, id: &str, asset: Option<&str>) -> Result<u64> {
        let db = self.executor.producer.database.clone();

        let contract_id = id.parse().unwrap();

        let asset_id = match asset {
            Some(asset) => asset.parse().unwrap(),
            None => AssetId::default(),
        };

        let result = db
            .storage::<ContractsAssets>()
            .get(&(&contract_id, &asset_id))
            .unwrap();
        let balance = result.unwrap_or_default().into_owned();

        Ok(balance)
    }

    pub fn previous_block_info(&self, height: BlockHeight) -> Result<PreviousBlockInfo> {
        // block 0 is reserved for genesis
        if height == 0u32.into() {
            Err(GenesisBlock.into())
        }
        // if this is the first block, fill in base metadata from genesis
        else if height == 1u32.into() {
            // TODO: what should initial genesis data be here?
            Ok(PreviousBlockInfo {
                prev_root: Default::default(),
                da_height: Default::default(),
            })
        } else {
            // get info from previous block height
            let prev_height = height - 1u32.into();
            let previous_block = self
                .executor
                .producer
                .database
                .get_block(prev_height)?
                .ok_or(MissingBlock(prev_height))?;
            // TODO: this should use a proper BMT MMR
            let hash = previous_block.id();
            let prev_root =
                ephemeral_merkle_root(vec![*previous_block.header.prev_root(), hash.into()].iter());

            Ok(PreviousBlockInfo {
                prev_root,
                da_height: previous_block.header.da_height,
            })
        }
    }
}
