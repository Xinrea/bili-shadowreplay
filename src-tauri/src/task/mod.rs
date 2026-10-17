use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{interval, Duration};

/// Task execution function type
pub type TaskFn = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'static>>;

/// Task status enumeration
#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

/// Task priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(dead_code)]
pub enum TaskPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Represents a single task
#[allow(dead_code)]
pub struct Task {
    pub task_id: String,
    pub priority: TaskPriority,
    pub status: TaskStatus,
    pub created_at: std::time::Instant,
    task_fn: Option<TaskFn>,
    cancel_handle: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Task {
    pub fn new<F>(task_id: String, priority: TaskPriority, task_fn: F) -> Self
    where
        F: Future<Output = Result<(), String>> + Send + 'static,
    {
        Task {
            task_id,
            priority,
            status: TaskStatus::Pending,
            created_at: std::time::Instant::now(),
            task_fn: Some(Box::pin(task_fn)),
            cancel_handle: None,
        }
    }

    #[allow(dead_code)]
    pub fn new_with_default_priority<F>(task_id: String, task_fn: F) -> Self
    where
        F: Future<Output = Result<(), String>> + Send + 'static,
    {
        Self::new(task_id, TaskPriority::Normal, task_fn)
    }
}

/// Task manager configuration
#[derive(Debug, Clone)]
pub struct TaskManagerConfig {
    /// Maximum number of tasks that can run concurrently
    pub max_concurrent_tasks: usize,
    /// Interval for checking the queue (in milliseconds)
    pub check_interval_ms: u64,
    /// Maximum queue size (0 = unlimited)
    pub max_queue_size: usize,
}

impl Default for TaskManagerConfig {
    fn default() -> Self {
        TaskManagerConfig {
            max_concurrent_tasks: 1,
            check_interval_ms: 1000,
            max_queue_size: 20,
        }
    }
}

/// Main task manager structure
pub struct TaskManager {
    config: TaskManagerConfig,
    queue: Arc<Mutex<VecDeque<Task>>>,
    running_tasks: Arc<RwLock<HashMap<String, JoinHandle<()>>>>,
    task_statuses: Arc<RwLock<HashMap<String, TaskStatus>>>,
    scheduler_handle: Option<JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
}

impl TaskManager {
    /// Create a new task manager with default configuration
    pub fn new() -> Self {
        Self::with_config(TaskManagerConfig::default())
    }

    /// Create a new task manager with custom configuration
    pub fn with_config(config: TaskManagerConfig) -> Self {
        TaskManager {
            config,
            queue: Arc::new(Mutex::new(VecDeque::new())),
            running_tasks: Arc::new(RwLock::new(HashMap::new())),
            task_statuses: Arc::new(RwLock::new(HashMap::new())),
            scheduler_handle: None,
            shutdown_tx: None,
        }
    }

    /// Start the task manager's scheduler
    pub fn start(&mut self) {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        self.shutdown_tx = Some(shutdown_tx.clone());

        let queue = Arc::clone(&self.queue);
        let running_tasks = Arc::clone(&self.running_tasks);
        let task_statuses = Arc::clone(&self.task_statuses);
        let config = self.config.clone();
        let mut shutdown_rx = shutdown_tx.subscribe();

        let handle = tokio::spawn(async move {
            let mut check_interval = interval(Duration::from_millis(config.check_interval_ms));

            loop {
                tokio::select! {
                    _ = check_interval.tick() => {
                        Self::process_queue(
                            &queue,
                            &running_tasks,
                            &task_statuses,
                            config.max_concurrent_tasks,
                        )
                        .await;
                    }
                    _ = shutdown_rx.recv() => {
                        break;
                    }
                }
            }
        });

        self.scheduler_handle = Some(handle);
    }

    /// Stop the task manager's scheduler
    #[allow(dead_code)]
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.scheduler_handle.take() {
            let _ = handle.await;
        }

        // Cancel all running tasks
        let running_tasks = self.running_tasks.read().await;
        for handle in running_tasks.values() {
            handle.abort();
        }
    }

    /// Add a task to the queue
    pub async fn add_task(&self, task: Task) -> Result<(), String> {
        let mut queue = self.queue.lock().await;
        let mut statuses = self.task_statuses.write().await;

        if matches!(
            statuses.get(&task.task_id),
            Some(TaskStatus::Pending | TaskStatus::Running)
        ) {
            return Err("Task already exists".to_string());
        }

        if self.config.max_queue_size > 0 && queue.len() >= self.config.max_queue_size {
            return Err("Queue is full".to_string());
        }

        let task_id = task.task_id.clone();

        // Insert task based on priority
        let insert_pos = queue
            .iter()
            .position(|t| t.priority < task.priority)
            .unwrap_or(queue.len());

        queue.insert(insert_pos, task);

        // Update task status
        statuses.insert(task_id, TaskStatus::Pending);

        Ok(())
    }

    /// Cancel a task by ID
    pub async fn cancel_task(&self, task_id: &str) -> Result<(), String> {
        // Use the scheduler's lock order so a task cannot move from the queue
        // to the running map between these checks.
        let mut queue = self.queue.lock().await;
        let mut running_tasks = self.running_tasks.write().await;
        if let Some(handle) = running_tasks.remove(task_id) {
            handle.abort();
            // Keep the slot locked until the future (and its owned FFmpeg
            // child with kill_on_drop) has been dropped. No progress writes
            // from this task can race the caller's cancelled status afterward.
            let _ = handle.await;
            let mut statuses = self.task_statuses.write().await;
            statuses.insert(task_id.to_string(), TaskStatus::Cancelled);
            return Ok(());
        }

        let pos = queue.iter().position(|t| t.task_id == task_id);
        if let Some(pos) = pos {
            queue.remove(pos);
            let mut statuses = self.task_statuses.write().await;
            statuses.insert(task_id.to_string(), TaskStatus::Cancelled);
            return Ok(());
        }

        Err("Task not found".to_string())
    }

    /// Get the status of a task
    #[allow(dead_code)]
    pub async fn get_task_status(&self, task_id: &str) -> Option<TaskStatus> {
        let statuses = self.task_statuses.read().await;
        statuses.get(task_id).cloned()
    }

    /// Get all task statuses
    #[allow(dead_code)]
    pub async fn get_all_task_statuses(&self) -> HashMap<String, TaskStatus> {
        let statuses = self.task_statuses.read().await;
        statuses.clone()
    }

    /// Get the number of tasks in queue
    #[allow(dead_code)]
    pub async fn queue_size(&self) -> usize {
        let queue = self.queue.lock().await;
        queue.len()
    }

    /// Get the number of running tasks
    #[allow(dead_code)]
    pub async fn running_count(&self) -> usize {
        let running = self.running_tasks.read().await;
        running.len()
    }

    /// Clear all completed and failed tasks from status tracking
    #[allow(dead_code)]
    pub async fn clear_finished_tasks(&self) {
        let mut statuses = self.task_statuses.write().await;
        statuses.retain(|_, status| {
            !matches!(
                status,
                TaskStatus::Completed | TaskStatus::Failed(_) | TaskStatus::Cancelled
            )
        });
    }

    /// Process the queue and start tasks if slots are available
    async fn process_queue(
        queue: &Arc<Mutex<VecDeque<Task>>>,
        running_tasks: &Arc<RwLock<HashMap<String, JoinHandle<()>>>>,
        task_statuses: &Arc<RwLock<HashMap<String, TaskStatus>>>,
        max_concurrent: usize,
    ) {
        let mut queue = queue.lock().await;
        let mut running = running_tasks.write().await;
        // Reclaim panicked/aborted workers before testing capacity.
        running.retain(|_, handle| !handle.is_finished());
        if running.len() >= max_concurrent {
            return;
        }

        if let Some(mut task) = queue.pop_front() {
            let task_id = task.task_id.clone();

            // Update status to running
            {
                let mut statuses = task_statuses.write().await;
                statuses.insert(task_id.clone(), TaskStatus::Running);
            }

            // Take the task function
            if let Some(task_fn) = task.task_fn.take() {
                let task_id_clone = task_id.clone();
                let running_tasks_clone = Arc::clone(running_tasks);
                let task_statuses_clone = Arc::clone(task_statuses);

                // Spawn the task
                let handle = tokio::spawn(async move {
                    let result = task_fn.await;

                    // Match the scheduler/cancellation lock order. Registration
                    // below holds this lock so even an immediate completion
                    // cannot remove itself before its handle is registered.
                    let mut running = running_tasks_clone.write().await;
                    let mut statuses = task_statuses_clone.write().await;
                    match result {
                        Ok(_) => {
                            statuses.insert(task_id_clone.clone(), TaskStatus::Completed);
                        }
                        Err(e) => {
                            statuses.insert(task_id_clone.clone(), TaskStatus::Failed(e));
                        }
                    }

                    // Remove from running tasks
                    running.remove(&task_id_clone);
                });

                // Add to running tasks
                running.insert(task_id, handle);
            }
        }
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TaskManager {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::{mpsc, oneshot};
    use tokio::time::{sleep, timeout};

    fn fast_manager(max_queue_size: usize) -> TaskManager {
        TaskManager::with_config(TaskManagerConfig {
            max_concurrent_tasks: 1,
            check_interval_ms: 5,
            max_queue_size,
        })
    }

    #[tokio::test]
    async fn serial_fifo_and_pending_cancellation() {
        let mut manager = fast_manager(20);
        let (events_tx, mut events) = mpsc::unbounded_channel();
        let (release_tx, release_rx) = oneshot::channel();
        let first_tx = events_tx.clone();
        manager
            .add_task(Task::new(
                "first".into(),
                TaskPriority::Normal,
                async move {
                    first_tx.send("first started").unwrap();
                    let _ = release_rx.await;
                    first_tx.send("first finished").unwrap();
                    Ok(())
                },
            ))
            .await
            .unwrap();
        for id in ["second", "cancel-pending", "third"] {
            let tx = events_tx.clone();
            manager
                .add_task(Task::new(id.into(), TaskPriority::Normal, async move {
                    tx.send(id).unwrap();
                    Ok(())
                }))
                .await
                .unwrap();
        }
        manager.start();
        assert_eq!(
            timeout(Duration::from_secs(1), events.recv())
                .await
                .unwrap(),
            Some("first started")
        );
        assert_eq!(manager.running_count().await, 1);
        assert_eq!(manager.queue_size().await, 3);
        manager.cancel_task("cancel-pending").await.unwrap();
        assert_eq!(
            manager.get_task_status("cancel-pending").await,
            Some(TaskStatus::Cancelled)
        );
        assert!(timeout(Duration::from_millis(25), events.recv())
            .await
            .is_err());
        release_tx.send(()).unwrap();
        for expected in ["first finished", "second", "third"] {
            assert_eq!(
                timeout(Duration::from_secs(1), events.recv())
                    .await
                    .unwrap(),
                Some(expected)
            );
        }
        manager.stop().await;
    }

    #[tokio::test]
    async fn cancellation_waits_for_worker_drop_and_next_job_runs() {
        let mut manager = fast_manager(20);
        let (started_tx, started_rx) = oneshot::channel();
        let (dropped_tx, mut dropped_rx) = oneshot::channel();
        let (next_tx, next_rx) = oneshot::channel();
        struct DropSignal(Option<oneshot::Sender<()>>);
        impl Drop for DropSignal {
            fn drop(&mut self) {
                if let Some(tx) = self.0.take() {
                    let _ = tx.send(());
                }
            }
        }
        manager
            .add_task(Task::new(
                "running".into(),
                TaskPriority::Normal,
                async move {
                    let _guard = DropSignal(Some(dropped_tx));
                    let _ = started_tx.send(());
                    std::future::pending::<()>().await;
                    Ok(())
                },
            ))
            .await
            .unwrap();
        manager
            .add_task(Task::new("next".into(), TaskPriority::Normal, async move {
                let _ = next_tx.send(());
                Ok(())
            }))
            .await
            .unwrap();
        manager.start();
        timeout(Duration::from_secs(1), started_rx)
            .await
            .unwrap()
            .unwrap();
        manager.cancel_task("running").await.unwrap();
        // Cancellation must not return before owned resources are released.
        assert_eq!(dropped_rx.try_recv(), Ok(()));
        timeout(Duration::from_secs(1), next_rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            manager.get_task_status("running").await,
            Some(TaskStatus::Cancelled)
        );
        manager.stop().await;
    }

    #[tokio::test]
    async fn full_queue_and_duplicate_ids_are_rejected_without_losing_jobs() {
        let manager = fast_manager(1);
        manager
            .add_task(Task::new("first".into(), TaskPriority::Normal, async {
                Ok(())
            }))
            .await
            .unwrap();
        assert_eq!(
            manager
                .add_task(Task::new("first".into(), TaskPriority::Normal, async {
                    Ok(())
                }))
                .await
                .unwrap_err(),
            "Task already exists"
        );
        assert_eq!(
            manager
                .add_task(Task::new("second".into(), TaskPriority::Normal, async {
                    Ok(())
                }))
                .await
                .unwrap_err(),
            "Queue is full"
        );
        assert_eq!(manager.queue_size().await, 1);
        manager.cancel_task("first").await.unwrap();
        manager
            .add_task(Task::new("second".into(), TaskPriority::Normal, async {
                Ok(())
            }))
            .await
            .unwrap();
        assert_eq!(manager.queue_size().await, 1);
    }

    #[tokio::test]
    async fn test_task_manager_basic() {
        let mut manager = TaskManager::new();
        manager.start();

        let task = Task::new("test-1".to_string(), TaskPriority::Normal, async {
            sleep(Duration::from_millis(100)).await;
            Ok(())
        });

        manager.add_task(task).await.unwrap();
        sleep(Duration::from_millis(200)).await;

        let status = manager.get_task_status("test-1").await;
        assert_eq!(status, Some(TaskStatus::Completed));

        manager.stop().await;
    }

    #[tokio::test]
    async fn test_task_cancellation() {
        let mut manager = TaskManager::new();
        manager.start();

        let task = Task::new("test-cancel".to_string(), TaskPriority::Normal, async {
            sleep(Duration::from_secs(10)).await;
            Ok(())
        });

        manager.add_task(task).await.unwrap();
        sleep(Duration::from_millis(100)).await;

        manager.cancel_task("test-cancel").await.unwrap();
        let status = manager.get_task_status("test-cancel").await;
        assert_eq!(status, Some(TaskStatus::Cancelled));

        manager.stop().await;
    }

    #[tokio::test]
    async fn test_task_priority() {
        let mut manager = TaskManager::with_config(TaskManagerConfig {
            max_concurrent_tasks: 1,
            check_interval_ms: 100,
            max_queue_size: 10,
        });
        manager.start();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let low_tx = tx.clone();

        // Add low priority task
        let task1 = Task::new("low".to_string(), TaskPriority::Low, async move {
            low_tx.send("low").unwrap();
            Ok(())
        });

        // Add high priority task
        let task2 = Task::new("high".to_string(), TaskPriority::High, async move {
            tx.send("high").unwrap();
            Ok(())
        });

        manager.add_task(task1).await.unwrap();
        manager.add_task(task2).await.unwrap();

        assert_eq!(
            timeout(Duration::from_secs(1), rx.recv()).await.unwrap(),
            Some("high")
        );
        assert_eq!(
            timeout(Duration::from_secs(1), rx.recv()).await.unwrap(),
            Some("low")
        );

        manager.stop().await;
    }
}
