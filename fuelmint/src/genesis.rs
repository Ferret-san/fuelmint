use anyhow::anyhow;
use fuel_core::{database::Database, service::config::Config};
use fuel_core_chain_config::{ContractConfig, GenesisCommitment, StateConfig};
use fuel_core_executor::refs::ContractRef;
use fuel_core_poa::ports::BlockDb;
use fuel_core_storage::{
    tables::{
        Coins, ContractsAssets, ContractsInfo, ContractsLatestUtxo, ContractsRawCode,
        ContractsState, FuelBlocks, Messages,
    },
    transactional::Transaction,
    MerkleRoot, StorageAsMut,
};
use fuel_core_types::{
    blockchain::{
        block::Block,
        consensus::{Consensus, Genesis},
        header::{ApplicationHeader, ConsensusHeader, PartialBlockHeader},
        primitives::Empty,
    },
    entities::{
        coin::{CoinStatus, CompressedCoin},
        message::Message,
    },
    fuel_merkle::binary,
    fuel_tx::{Contract, MessageId, UtxoId},
    fuel_types::{bytes::WORD_SIZE, Bytes32, ContractId},
};
use itertools::Itertools;

pub(crate) fn initialize_state(config: &Config, database: &Database) -> anyhow::Result<()> {
    if database.get_chain_name()?.is_none() {
        // start a db transaction for bulk-writing
        let mut import_tx = database.transaction();
        let database = import_tx.as_mut();

        add_genesis_block(config, database)?;

        // Write transaction to db
        import_tx.commit()?;
    }

    Ok(())
}

fn add_genesis_block(config: &Config, database: &mut Database) -> anyhow::Result<()> {
    // Initialize the chain id and height.
    database.init(&config.chain_conf)?;
    let chain_config_hash = config.chain_conf.clone().root()?.into();
    let coins_root = init_coin_state(database, &config.chain_conf.initial_state)?.into();
    let contracts_root = init_contracts(database, &config.chain_conf.initial_state)?.into();
    let (messages_root, message_ids) =
        init_da_messages(database, &config.chain_conf.initial_state)?;
    let messages_root = messages_root.into();

    let genesis = Genesis {
        chain_config_hash,
        coins_root,
        contracts_root,
        messages_root,
    };

    let block = Block::new(
        PartialBlockHeader {
            application: ApplicationHeader::<Empty> {
                da_height: Default::default(),
                generated: Empty,
            },
            consensus: ConsensusHeader::<Empty> {
                // The genesis is a first block, so previous root is zero.
                prev_root: Bytes32::zeroed(),
                // The initial height is defined by the `ChainConfig`.
                // If it is `None` then it will be zero.
                height: config
                    .chain_conf
                    .initial_state
                    .as_ref()
                    .map(|config| config.height.unwrap_or_else(|| 0u32.into()))
                    .unwrap_or_else(|| 0u32.into()),
                time: fuel_core_types::tai64::Tai64::UNIX_EPOCH,
                generated: Empty,
            },
            metadata: None,
        },
        // Genesis block doesn't have any transaction.
        vec![],
        &message_ids,
    );

    let seal = Consensus::Genesis(genesis);
    let block_id = block.id();
    database
        .storage::<FuelBlocks>()
        .insert(&block_id, &block.compress())?;
    database.seal_block(block_id, seal)
}

fn init_coin_state(db: &mut Database, state: &Option<StateConfig>) -> anyhow::Result<MerkleRoot> {
    let mut coins_tree = binary::in_memory::MerkleTree::new();
    // TODO: Store merkle sum tree root over coins with unspecified utxo ids.
    let mut generated_output_index: u64 = 0;
    if let Some(state) = &state {
        if let Some(coins) = &state.coins {
            for coin in coins {
                let utxo_id = UtxoId::new(
                    // generated transaction id([0..[out_index/255]])
                    coin.tx_id.unwrap_or_else(|| {
                        Bytes32::try_from(
                            (0..(Bytes32::LEN - WORD_SIZE))
                                .map(|_| 0u8)
                                .chain((generated_output_index / 255).to_be_bytes().into_iter())
                                .collect_vec()
                                .as_slice(),
                        )
                        .expect("Incorrect genesis transaction id byte length")
                    }),
                    coin.output_index.map(|i| i as u8).unwrap_or_else(|| {
                        generated_output_index += 1;
                        (generated_output_index % 255) as u8
                    }),
                );

                let mut coin = CompressedCoin {
                    owner: coin.owner,
                    amount: coin.amount,
                    asset_id: coin.asset_id,
                    maturity: coin.maturity.unwrap_or_default(),
                    status: CoinStatus::Unspent,
                    block_created: coin.block_created.unwrap_or_default(),
                };

                if db.storage::<Coins>().insert(&utxo_id, &coin)?.is_some() {
                    return Err(anyhow!("Coin should not exist"));
                }
                coins_tree.push(coin.root()?.as_slice())
            }
        }
    }
    Ok(coins_tree.root())
}

fn init_contracts(db: &mut Database, state: &Option<StateConfig>) -> anyhow::Result<MerkleRoot> {
    let mut contracts_tree = binary::in_memory::MerkleTree::new();
    // initialize contract state
    if let Some(state) = &state {
        if let Some(contracts) = &state.contracts {
            for (generated_output_index, contract_config) in contracts.iter().enumerate() {
                let contract = Contract::from(contract_config.code.as_slice());
                let salt = contract_config.salt;
                let root = contract.root();
                let contract_id = contract.id(&salt, &root, &Contract::default_state_root());
                // insert contract code
                if db
                    .storage::<ContractsRawCode>()
                    .insert(&contract_id, contract.as_ref())?
                    .is_some()
                {
                    return Err(anyhow!("Contract code should not exist"));
                }

                // insert contract root
                if db
                    .storage::<ContractsInfo>()
                    .insert(&contract_id, &(salt, root))?
                    .is_some()
                {
                    return Err(anyhow!("Contract info should not exist"));
                }
                if db
                    .storage::<ContractsLatestUtxo>()
                    .insert(
                        &contract_id,
                        &UtxoId::new(
                            // generated transaction id([0..[out_index/255]])
                            Bytes32::try_from(
                                (0..(Bytes32::LEN - WORD_SIZE))
                                    .map(|_| 0u8)
                                    .chain(
                                        (generated_output_index as u64 / 255)
                                            .to_be_bytes()
                                            .into_iter(),
                                    )
                                    .collect_vec()
                                    .as_slice(),
                            )
                            .expect("Incorrect genesis transaction id byte length"),
                            generated_output_index as u8,
                        ),
                    )?
                    .is_some()
                {
                    return Err(anyhow!("Contract utxo should not exist"));
                }
                init_contract_state(db, &contract_id, contract_config)?;
                init_contract_balance(db, &contract_id, contract_config)?;
                contracts_tree.push(ContractRef::new(&mut *db, contract_id).root()?.as_slice());
            }
        }
    }
    Ok(contracts_tree.root())
}

fn init_contract_state(
    db: &mut Database,
    contract_id: &ContractId,
    contract: &ContractConfig,
) -> anyhow::Result<()> {
    // insert state related to contract
    if let Some(contract_state) = &contract.state {
        for (key, value) in contract_state {
            if db
                .storage::<ContractsState>()
                .insert(&(contract_id, key), value)?
                .is_some()
            {
                return Err(anyhow!("Contract state should not exist"));
            }
        }
    }
    Ok(())
}

fn init_da_messages(
    db: &mut Database,
    state: &Option<StateConfig>,
) -> anyhow::Result<(MerkleRoot, Vec<MessageId>)> {
    let mut message_tree = binary::in_memory::MerkleTree::new();
    let mut message_ids = vec![];
    if let Some(state) = &state {
        if let Some(message_state) = &state.messages {
            for msg in message_state {
                let mut message = Message {
                    sender: msg.sender,
                    recipient: msg.recipient,
                    nonce: msg.nonce,
                    amount: msg.amount,
                    data: msg.data.clone(),
                    da_height: msg.da_height,
                    fuel_block_spend: None,
                };

                let message_id = message.id();
                if db
                    .storage::<Messages>()
                    .insert(&message_id, &message)?
                    .is_some()
                {
                    return Err(anyhow!("Message should not exist"));
                }
                message_tree.push(message.root()?.as_slice());
                message_ids.push(message_id);
            }
        }
    }

    Ok((message_tree.root(), message_ids))
}

fn init_contract_balance(
    db: &mut Database,
    contract_id: &ContractId,
    contract: &ContractConfig,
) -> anyhow::Result<()> {
    // insert balances related to contract
    if let Some(balances) = &contract.balances {
        for (key, value) in balances {
            if db
                .storage::<ContractsAssets>()
                .insert(&(contract_id, key), value)?
                .is_some()
            {
                return Err(anyhow!("Contract balance should not exist"));
            }
        }
    }
    Ok(())
}
