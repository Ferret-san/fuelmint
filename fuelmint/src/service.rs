use crate::genesis::initialize_state;
use crate::sub_services;
use fuel_core::{
    database::Database,
    fuel_core_graphql_api,
    producer::Producer,
    service::{adapters::P2PAdapter, Config, ServiceTrait, SubServices},
};
use fuel_core_services::{RunnableService, RunnableTask, ServiceRunner, State, StateWatcher};
use std::{net::SocketAddr, panic, sync::Arc};

#[derive(Clone)]
pub struct SharedState {
    /// The transaction pool shared state.
    pub txpool: fuel_core_txpool::service::SharedState<P2PAdapter, Database>,
    /// The GraphQL shared state.
    pub graph_ql: fuel_core_graphql_api::service::SharedState,
}

pub struct FuelmintService {
    /// The `ServiceRunner` used for `FuelmintService`.
    runner: ServiceRunner<Task>,
    /// The shared state of the service
    pub shared: SharedState,
    /// The address bound by the system for serving the API
    pub bound_address: SocketAddr,
}

impl FuelmintService {
    /// Creates a `FuelService` instance from service config
    #[tracing::instrument(skip_all)]
    pub fn new(
        database: Database,
        config: Config,
    ) -> anyhow::Result<(Box<Arc<Producer<Database>>>, Self)> {
        let (block_producer, task) = Task::new(database, config)?;
        let runner = ServiceRunner::new(task);
        let shared = runner.shared.clone();
        let bound_address = runner.shared.graph_ql.bound_address;
        Ok((
            block_producer,
            FuelmintService {
                bound_address,
                shared,
                runner,
            },
        ))
    }

    /// Creates and starts fuel node instance from service config and a pre-existing database
    pub async fn from_database(
        database: Database,
        config: Config,
    ) -> anyhow::Result<(Box<Arc<Producer<Database>>>, Self)> {
        let (block_producer, service) = Self::new(database, config)?;
        Ok((block_producer, service))
    }
}

#[async_trait::async_trait]
impl ServiceTrait for FuelmintService {
    fn start(&self) -> anyhow::Result<()> {
        self.runner.start()
    }

    async fn start_and_await(&self) -> anyhow::Result<State> {
        self.runner.start_and_await().await
    }

    async fn await_start_or_stop(&self) -> anyhow::Result<State> {
        self.runner.await_start_or_stop().await
    }

    fn stop(&self) -> bool {
        self.runner.stop()
    }

    async fn stop_and_await(&self) -> anyhow::Result<State> {
        self.runner.stop_and_await().await
    }

    async fn await_stop(&self) -> anyhow::Result<State> {
        self.runner.await_stop().await
    }

    fn state(&self) -> State {
        self.runner.state()
    }
}

pub struct Task {
    /// The list of started sub services.
    services: SubServices,
    /// The address bound by the system for serving the API
    pub shared: SharedState,
}

impl Task {
    /// Private inner method for initializing the fuel service task
    pub fn new(
        database: Database,
        config: Config,
    ) -> anyhow::Result<(Box<Arc<Producer<Database>>>, Task)> {
        // Initialize the state of the chain
        initialize_state(&config, &database)?;
        // initialize sub services
        let (block_producer, services, shared) =
            sub_services::init_sub_services(&config, &database)?;
        // initialize sub services
        Ok((block_producer, Task { services, shared }))
    }

    #[cfg(test)]
    pub fn sub_services(&mut self) -> &mut SubServices {
        &mut self.services
    }
}

#[async_trait::async_trait]
impl RunnableService for Task {
    const NAME: &'static str = "FuelService";
    type SharedData = SharedState;
    type Task = Task;

    fn shared_data(&self) -> Self::SharedData {
        self.shared.clone()
    }

    async fn into_task(self, _: &StateWatcher) -> anyhow::Result<Self::Task> {
        for service in &self.services {
            service.start_and_await().await?;
        }
        Ok(self)
    }
}

// its possible that the Service Runner is not aware of our Task, and thus
// the task stops immediatly
// TODO: test with local fuel-core

#[async_trait::async_trait]
impl RunnableTask for Task {
    async fn run(&mut self, watcher: &mut StateWatcher) -> anyhow::Result<bool> {
        println!("Running Fuelmint Task!");
        let mut stop_signals = vec![];
        for service in &self.services {
            stop_signals.push(service.await_stop())
        }
        stop_signals.push(Box::pin(shutdown_signal()));
        stop_signals.push(Box::pin(watcher.while_started()));

        let (result, _, _) = futures::future::select_all(stop_signals).await;

        if let Err(err) = result {
            tracing::error!("Got an error during listen for shutdown: {}", err);
        }

        // We received the stop signal from any of one source, so stop this service and
        // all sub-services.
        for service in &self.services {
            let result = service.stop_and_await().await;

            if let Err(err) = result {
                tracing::error!(
                    "Got and error during awaiting for stop of the service: {}",
                    err
                );
            }
        }

        Ok(false /* should_continue */)
    }
}

async fn shutdown_signal() -> anyhow::Result<State> {
    #[cfg(unix)]
    {
        println!("Received a shutdown_sginal");
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        loop {
            tokio::select! {
                _ = sigterm.recv() => {
                    tracing::info!("sigterm received");
                    break;
                }
                _ = sigint.recv() => {
                    tracing::log::info!("sigint received");
                    break;
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
        tracing::log::info!("CTRL+C received");
    }
    Ok(State::Stopped)
}
