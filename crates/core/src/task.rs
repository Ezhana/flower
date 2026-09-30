use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};
use crossbeam_deque::Injector;
use futures::future::BoxFuture;
use futures::task::ArcWake;

struct Task{
    future: Mutex<Option<BoxFuture<'static, ()>>>,
    state: AtomicU8,
    scheduler: Arc<Scheduler>
}

impl ArcWake for Task {
    fn wake_by_ref(task: &Arc<Self>) {
        task.scheduler();
    }
}

impl Task{
    fn new<F>(future: F, scheduler: Arc<Scheduler>) -> Arc<Self>
    where F: Future<Output = ()> + Send+'static,{
        Arc::new(Self{
            future: Mutex::new(Some(Box::pin(future))),
            state: AtomicU8::new(IDLE),
            scheduler,
        })
    }
}



type Runnable=Arc<Task>;

struct Scheduler{
    global: Arc<Injector<Runnable>>
}

impl Scheduler {
    fn schedule(&self, task: Runnable) {
        self.global.push(task);
    }
}

const IDLE: u8 = 0;
const QUEUED: u8 = 1;
const RUNNING: u8 = 2;
const NOTIFIED: u8 = 3;
const COMPLETE: u8 = 4;