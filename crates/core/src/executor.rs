use std::sync::Arc;
use std::thread::JoinHandle;

struct Executor {
    scheduler: Arc<Scheduler>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    workers: Vec<JoinHandle<()>>,
}