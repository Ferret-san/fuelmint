use std::sync::Arc;
use tokio::sync::Mutex;

use abci::{
    async_api::{
        Consensus as ConsensusTrait, Info as InfoTrait, Mempool as MempoolTrait,
        Snapshot as SnapshotTrait,
    },
    async_trait,
    types::*,
};

// TODO: Import Fuel equivalents for CacheDb, EmptyDB

/// The app's state, containing a Rocks DB.
// TODO: replace this with Fuel Core and implement traits for it?
#[derive(Clone, Debug)]
pub struct State<Db> {
    pub block_height: i64,
    pub app_hash: Vec<u8>,
    pub db: Db,
    //pub env: Env,
}

// impl Default for State<Db> {
//     fn default() -> Self {
//         Self {
//             block_height: 0,
//             app_hash: Vec::new(),
//             // db: CacheDB::new(EmptyDB()),
//             //env: Default::default(),
//         }
//     }
// }

// impl<Db: Database + DatabaseCommit> State<Db> {
//     async fn execute(
//         &mut self,
//         tx: TransactionRequest,
//         read_only: bool,
//     ) -> eyre::Result<TransactionResult> {
//         let mut evm = revm::EVM::new();
//         evm.env = self.env.clone();
//         evm.env.tx = TxEnv {
//             caller: tx.from.unwrap_or_default(),
//             transact_to: match tx.to {
//                 Some(NameOrAddress::Address(inner)) => TransactTo::Call(inner),
//                 Some(NameOrAddress::Name(_)) => panic!("not allowed"),
//                 None => TransactTo::Create(CreateScheme::Create),
//             },
//             data: tx.data.clone().unwrap_or_default().0,
//             chain_id: Some(self.env.cfg.chain_id.as_u64()),
//             nonce: Some(tx.nonce.unwrap_or_default().as_u64()),
//             value: tx.value.unwrap_or_default(),
//             gas_price: tx.gas_price.unwrap_or_default(),
//             gas_priority_fee: Some(tx.gas_price.unwrap_or_default()),
//             gas_limit: tx.gas.unwrap_or_default().as_u64(),
//             access_list: vec![],
//         };
//         evm.database(&mut self.db);

//         let (ret, out, gas, state, logs) = evm.transact();
//         if !read_only {
//             self.db.commit(state);
//         };

//         Ok(TransactionResult {
//             transaction: tx,
//             exit: ret,
//             gas,
//             logs,
//             out,
//         })
//     }
// }

pub struct Consensus<Db> {
    pub committed_state: Arc<Mutex<State<Db>>>,
    pub current_state: Arc<Mutex<State<Db>>>,
}

impl<Db: Clone> Consensus<Db> {
    pub fn new(state: State<Db>) -> Self {
        let committed_state = Arc::new(Mutex::new(state.clone()));
        let current_state = Arc::new(Mutex::new(state));

        Consensus {
            committed_state,
            current_state,
        }
    }
}

// TODO: Figure out what to do with the DatabaseCommit and Database
#[async_trait]
impl<Db: Clone + Send + Sync + DatabaseCommit + Database> ConsensusTrait for Consensus<Db> {
    #[tracing::instrument(skip(self))]
    async fn init_chain(&self, _init_chain_request: RequestInitChain) -> ResponseInitChain {
        ResponseInitChain::default()
    }

    #[tracing::instrument(skip(self))]
    async fn begin_block(&self, _begin_block_request: RequestBeginBlock) -> ResponseBeginBlock {
        ResponseBeginBlock::default()
    }

    #[tracing::instrument(skip(self))]
    async fn deliver_tx(&self, deliver_tx_request: RequestDeliverTx) -> ResponseDeliverTx {
        // TODO
    }

    #[tracing::instrument(skip(self))]
    async fn end_block(&self, end_block_request: RequestEndBlock) -> ResponseEndBlock {
        tracing::trace!("ending block");
        let mut current_state = self.current_state.lock().await;
        current_state.block_height = end_block_request.height;
        current_state.app_hash = vec![];
        tracing::trace!("done");

        ResponseEndBlock::default()
    }

    #[tracing::instrument(skip(self))]
    async fn commit(&self, _commit_request: RequestCommit) -> ResponseCommit {
        tracing::trace!("taking lock");
        let current_state = self.current_state.lock().await.clone();
        let mut committed_state = self.committed_state.lock().await;
        *committed_state = current_state;
        tracing::trace!("committed");

        ResponseCommit {
            data: vec![], // (*committed_state).app_hash.clone(),
            retain_height: 0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Mempool;

#[async_trait]
impl MempoolTrait for Mempool {
    async fn check_tx(&self, _check_tx_request: RequestCheckTx) -> ResponseCheckTx {
        ResponseCheckTx::default()
    }
}

#[derive(Debug, Clone)]
pub struct Info<Db> {
    pub state: Arc<Mutex<State<Db>>>,
}

// TODO: Implement Query and Snapshot
