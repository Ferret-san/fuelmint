use anyhow::{anyhow, Context};
use clap::Parser;
use fuel_core::{
    chain_config::ChainConfig,
    database::Database,
    producer::Config as ProducerConfig,
    service::{
        config::{default_consensus_dev_key, Config, DbType, VMConfig},
        ServiceTrait,
    },
    txpool::Config as TxPoolConfig,
    types::{
        blockchain::primitives::SecretKeyWrapper,
        fuel_tx::Address,
        fuel_vm::SecretKey,
        secrecy::{ExposeSecret, Secret},
    },
};
use fuelmint::{service, state::State, types::App};
use lazy_static::__Deref;
use std::str::FromStr;
use std::{
    env,
    net::{IpAddr, SocketAddr},
    panic,
    path::PathBuf,
};
use strum::VariantNames;
use tower::ServiceBuilder;
use tower_abci::{split, Server};
use tracing::log::warn;

pub const CONSENSUS_KEY_ENV: &str = "CONSENSUS_KEY_SECRET";

lazy_static::lazy_static! {
    pub static ref DEFAULT_DB_PATH: PathBuf = dirs::home_dir().unwrap().join(".fuel").join("db");
}

#[derive(Debug, Clone, Parser)]
pub struct Command {
    /// IP Address to run the server on
    #[clap(long = "ip", default_value = "127.0.0.1", parse(try_from_str))]
    pub ip: IpAddr,

    /// port to run the graph_ql client on
    #[clap(long = "port", default_value = "4000")]
    pub port: u16,

    /// Specify the port where the ABCI server will run on
    #[clap(long = "abci-port", default_value = "26658")]
    pub abci_port: u16,

    #[clap(
        name = "DB_PATH",
        long = "db-path",
        parse(from_os_str),
        default_value = (*DEFAULT_DB_PATH).to_str().unwrap()
    )]
    pub database_path: PathBuf,

    /// Type of database to use (InMemory or RocksDB)
    #[clap(long = "db-type", default_value = "rocks-db", possible_values = &*DbType::VARIANTS, ignore_case = true)]
    pub database_type: DbType,

    /// Specify either an alias to a built-in configuration or filepath to a JSON file.
    #[clap(name = "CHAIN_CONFIG", long = "chain", default_value = "local_testnet")]
    pub chain_config: String,

    /// Allows GraphQL Endpoints to arbitrarily advanced blocks. Should be used for local development only
    #[clap(long = "manual_blocks_enabled")]
    pub manual_blocks_enabled: bool,

    /// Enable logging of backtraces from vm errors
    #[clap(long = "vm-backtrace")]
    pub vm_backtrace: bool,

    /// Enable full utxo stateful validation
    /// disabled by default until downstream consumers stabilize
    #[clap(long = "utxo-validation")]
    pub utxo_validation: bool,

    /// The minimum allowed gas price
    #[clap(long = "min-gas-price", default_value = "0")]
    pub min_gas_price: u64,

    /// The signing key used when producing blocks.
    /// Setting via the `CONSENSUS_KEY_SECRET` ENV var is preferred.
    #[clap(long = "consensus-key")]
    pub consensus_key: Option<String>,

    /// Use a default insecure consensus key for testing purposes.
    /// This will not be enabled by default in the future.
    #[clap(long = "dev-keys", default_value = "true")]
    pub consensus_dev_key: bool,

    /// The block's fee recipient public key.
    ///
    /// If not set, `consensus_key` is used as the provider of the `Address`.
    #[clap(long = "coinbase-recipient")]
    pub coinbase_recipient: Option<String>,

    #[clap(long = "metrics")]
    pub metrics: bool,
}

impl Command {
    pub fn get_config(self) -> anyhow::Result<Config> {
        let Command {
            ip,
            port,
            abci_port: _,
            database_path,
            database_type,
            chain_config,
            vm_backtrace,
            manual_blocks_enabled,
            utxo_validation,
            min_gas_price,
            consensus_key,
            consensus_dev_key,
            coinbase_recipient,
            metrics,
        } = self;

        let addr = SocketAddr::new(ip, port);

        let chain_conf: ChainConfig = chain_config.as_str().parse()?;

        #[cfg(feature = "p2p")]
        let p2p_cfg = p2p_args.into_config(metrics)?;

        // if consensus key is not configured, fallback to dev consensus key
        let consensus_key = load_consensus_key(consensus_key)?.or_else(|| {
            if consensus_dev_key {
                let key = default_consensus_dev_key();
                warn!(
                    "Fuel Core is using an insecure test key for consensus. Public key: {}",
                    key.public_key()
                );
                Some(Secret::new(key.into()))
            } else {
                // if consensus dev key is disabled, use no key
                None
            }
        });

        let coinbase_recipient = if let Some(coinbase_recipient) = coinbase_recipient {
            Address::from_str(coinbase_recipient.as_str()).map_err(|err| anyhow!(err))?
        } else {
            let consensus_key = consensus_key
                .as_ref()
                .cloned()
                .unwrap_or_else(|| Secret::new(SecretKeyWrapper::default()));

            let sk = consensus_key.expose_secret().deref();
            Address::from(*sk.public_key().hash())
        };

        Ok(Config {
            addr,
            database_path,
            database_type,
            chain_conf: chain_conf.clone(),
            utxo_validation,
            manual_blocks_enabled,
            vm: VMConfig {
                backtrace: vm_backtrace,
            },
            txpool: TxPoolConfig::new(chain_conf, min_gas_price, utxo_validation),
            block_producer: ProducerConfig {
                utxo_validation,
                coinbase_recipient,
                metrics,
            },
            block_executor: Default::default(),
            consensus_key,
        })
    }
}

fn load_consensus_key(cli_arg: Option<String>) -> anyhow::Result<Option<Secret<SecretKeyWrapper>>> {
    let secret_string = if let Some(cli_arg) = cli_arg {
        warn!("Consensus key configured insecurely using cli args. Consider setting the {} env var instead.", CONSENSUS_KEY_ENV);
        Some(cli_arg)
    } else {
        env::var(CONSENSUS_KEY_ENV).ok()
    };

    if let Some(key) = secret_string {
        let key = SecretKey::from_str(&key).context("failed to parse consensus signing key")?;
        Ok(Some(Secret::new(key.into())))
    } else {
        Ok(None)
    }
}

#[derive(Parser, Debug)]
#[clap(
    name = "fuelmint",
    about = "Fuelmint ABCI client",
    version,
    rename_all = "kebab-case"
)]
pub struct Opt {
    #[clap(subcommand)]
    command: Fuelmint,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Parser)]
pub enum Fuelmint {
    Run(Command),
}

// TODO
// Add a way to define Config through a cli param
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let opt = Opt::try_parse();
    if opt.is_err() {
        let command = Command::try_parse();
        if let Ok(command) = command {
            warn!("This cli format for running `fuel-core` is deprecated and will be removed. Please use `fuel-core run` or use `--help` for more information");
            let res = exec(command).await;
            return res;
        }
    }

    match opt {
        Ok(opt) => match opt.command {
            Fuelmint::Run(command) => exec(command).await,
        },
        Err(e) => {
            // Prints the error and exits.
            e.exit()
        }
    }
}

async fn exec(command: Command) {
    let config = command.clone().get_config().unwrap();
    tracing::info!("Database path {:?}", config.database_path);
    let database = Database::open(&config.database_path).unwrap();
    // instantiate the FuelmintService
    let (block_producer, server) = service::FuelmintService::new(database, config.clone()).unwrap();
    let txpool = Box::new(server.shared.txpool.clone());

    // currently the stop signal is being sent immediatly, which shouldn't happen

    let state = State {
        block_height: 0,
        app_hash: Vec::new(),
    };

    // Construct our ABCI application.
    let service = App::new(config.clone(), state, block_producer, txpool);

    // Split it into components.
    let (consensus, mempool, snapshot, info) = split::service(service, 1);

    // Hand those components to the ABCI server, but customize request behavior
    // for each category -- for instance, apply load-shedding only to mempool
    // and info requests, but not to consensus requests.
    // Spawn a task to run the ABCI server
    let abci_server = tokio::task::spawn(
        Server::builder()
            .consensus(consensus)
            .snapshot(snapshot)
            .mempool(
                ServiceBuilder::new()
                    .load_shed()
                    .buffer(10)
                    .service(mempool),
            )
            .info(
                ServiceBuilder::new()
                    .load_shed()
                    .buffer(100)
                    .rate_limit(50, std::time::Duration::from_secs(1))
                    .service(info),
            )
            .finish()
            .unwrap()
            .listen(format!("{}:{}", command.ip, command.abci_port)),
    );

    tracing::info!(
        "ABCI server listening on {}::{}",
        command.ip,
        command.abci_port
    );

    let fuelmint_services = tokio::task::spawn(async move {
        server.start_and_await().await.unwrap();
        server.await_stop().await.unwrap();
    });

    tokio::select! {
        x = abci_server => x.unwrap().map_err(|e| anyhow::anyhow!(e)).unwrap(),
        x = fuelmint_services => x.unwrap()
    };
}
