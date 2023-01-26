use std::fmt::Debug;

use fuel_core_types::{
    blockchain::primitives::DaBlockHeight,
    fuel_tx::{Address, AssetId, ContractId},
};

use fuel_tx::Bytes32;

/// The application state, containing the block_height and the app_hash
#[derive(Clone, Debug)]
pub struct State {
    pub block_height: i64,
    pub app_hash: Vec<u8>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            block_height: 0,
            app_hash: Vec::new(),
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
