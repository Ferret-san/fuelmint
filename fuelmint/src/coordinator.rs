use fuel_core_services::{EmptyShared, RunnableService, RunnableTask, ServiceRunner, StateWatcher};
use fuel_core_types::blockchain::SealedBlock;
use tokio::sync::broadcast;

// A type of dummy "consensus" service that keeps the TxPool alive
// It essentially does nothing except preventing other services
// from shutting down
pub type Service = ServiceRunner<Task>;

pub struct Task {
    pub(crate) import_block_events_tx: broadcast::Sender<SealedBlock>,
}

impl Task {
    pub fn new(import_block_events_tx: broadcast::Sender<SealedBlock>) -> Self {
        Self {
            import_block_events_tx,
        }
    }
}

#[async_trait::async_trait]
impl RunnableService for Task
where
    Self: RunnableTask,
{
    const NAME: &'static str = "Coordinator";

    type SharedData = EmptyShared;
    type Task = Task;

    fn shared_data(&self) -> Self::SharedData {
        EmptyShared
    }

    async fn into_task(self, _: &StateWatcher) -> anyhow::Result<Self::Task> {
        Ok(self)
    }
}

#[async_trait::async_trait]
impl RunnableTask for Task {
    async fn run(&mut self, watcher: &mut StateWatcher) -> anyhow::Result<bool> {
        let should_continue;
        tokio::select! {
            // run while started
            _ = watcher.while_started() => {
                should_continue = false;
            }
        }
        Ok(should_continue)
    }
}

pub fn new_service(import_block_events_tx: broadcast::Sender<SealedBlock>) -> Service {
    Service::new(Task::new(import_block_events_tx))
}
