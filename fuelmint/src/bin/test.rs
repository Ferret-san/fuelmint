use anyhow::Result;
use fuel_core::schema::scalars::HexString;
use fuel_core_interfaces::common::{
    fuel_tx,
    fuel_vm::{consts::*, prelude::*},
};
use fuel_core_interfaces::common::{
    fuel_tx::{Cacheable, Transaction as FuelTx},
    fuel_vm::prelude::Deserializable,
};
use fuel_tx::Transaction;
use reqwest::Response;
use std::string::*;

async fn send_transaction(host: &str, tx: HexString) -> Result<Response> {
    let hex_string = &tx.to_string();
    let tx_string = hex_string.strip_prefix("0x").unwrap();
    println!("String: {:?}", hex_string);
    let mut tx = FuelTx::from_bytes(&hex::decode(tx_string).unwrap())?;
    tx.precompute();
    println!("Transaction: {:?}", tx);
    let client = reqwest::Client::new();
    let result = client
        .get(format!("{}/broadcast_tx", host))
        .query(&[("tx", tx_string)])
        .send()
        .await?;

    Ok(result)
}

// TODO
// Add function to initialize state from genesis if there's any
#[tokio::main]
async fn main() {
    let gas_price = 0;
    let gas_limit = 1_000_000;
    let maturity = 0;

    let script = vec![
        Opcode::ADDI(0x10, REG_ZERO, 0xca),
        Opcode::ADDI(0x11, REG_ZERO, 0xba),
        Opcode::LOG(0x10, 0x11, REG_ZERO, REG_ZERO),
        Opcode::RET(REG_ONE),
    ];
    let script: Vec<u8> = script
        .iter()
        .flat_map(|op| u32::from(*op).to_be_bytes())
        .collect();

    let mut tx: Transaction = fuel_tx::Transaction::script(
        gas_price,
        gas_limit,
        maturity,
        script,
        vec![],
        vec![],
        vec![],
        vec![],
    )
    .into();

    let hex_string = HexString::from(tx.to_bytes());
    print!("Hex string: {:?}", hex_string);
    let result = send_transaction("http://127.0.0.1:26657", hex_string)
        .await
        .unwrap();
    print!("Response: {:?}", result);
    // Use persistent storage to start the graphql service
    // let (stop_tx, stop_rx) = oneshot::channel::<()>();

    // let (bound_address, api_server) = start_server(
    //     service.current_state.executor.producer.config.clone(),
    //     service.current_state.executor.producer.database,
    //     stop_rx,
    // )
    // .await
    // .unwrap();
}
