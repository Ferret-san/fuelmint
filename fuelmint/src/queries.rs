use std::fmt::Debug;

use fuel_core_interfaces::common::prelude::{Address, AssetId, ContractId};

use fuel_tx::{Receipt, Transaction};

#[derive(serde::Serialize, serde::Deserialize, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Query {
    DryRun(Transaction),
    Balance(Address, AssetId),
    ContractBalance(ContractId, AssetId),
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum QueryResponse {
    Receipts(Vec<Receipt>),
    Balance(u64),
    ContractBalance(u64),
}

impl QueryResponse {
    pub fn as_receipts(&self) -> &Vec<Receipt> {
        match self {
            QueryResponse::Receipts(inner) => inner,
            _ => panic!("not receipts"),
        }
    }

    pub fn as_balance(&self) -> &u64 {
        match self {
            QueryResponse::Balance(inner) => inner,
            _ => panic!("not a balance"),
        }
    }

    pub fn as_contract_balance(&self) -> &u64 {
        match self {
            QueryResponse::ContractBalance(inner) => inner,
            _ => panic!("not a contract balance"),
        }
    }
}
