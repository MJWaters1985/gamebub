//! One background thread for longer jobs (Cartridge Manager, Wi-Fi, box art downloads), run
//! one at a time. It's started at boot: later the heap can be too fragmented to allocate a new
//! thread's stack.

use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;

type Job = Box<dyn FnOnce() + Send>;

static SENDER: Mutex<Option<Sender<Job>>> = Mutex::new(None);

pub fn start() {
    let (tx, rx) = channel::<Job>();
    *SENDER.lock().unwrap() = Some(tx);
    std::thread::Builder::new()
        .name("jobs".to_string())
        .stack_size(16 * 1024)
        .spawn(move || {
            while let Ok(job) = rx.recv() {
                job();
            }
        })
        .expect("start jobs thread");
}

/// Run a job on the background thread (after any job already running).
pub fn run(job: impl FnOnce() + Send + 'static) {
    if let Some(sender) = SENDER.lock().unwrap().as_ref() {
        let _ = sender.send(Box::new(job));
    }
}
