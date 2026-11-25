use super::*;
use crate::fs_win;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::time::Duration;

unsafe extern "system" {
    fn SetLastError(error: u32);
}

unsafe extern "system" fn no_event(_: Handle, _: i32, _: i32, _: *const u16) -> Handle {
    // SAFETY: SetLastError only changes the calling thread's error slot.
    unsafe { SetLastError(5) };
    core::ptr::null_mut()
}

unsafe extern "system" fn no_submission(_: Callback, _: Handle, _: Handle) -> i32 {
    // SAFETY: SetLastError only changes the calling thread's error slot.
    unsafe { SetLastError(5) };
    0
}

unsafe extern "system" fn no_wait(_: Handle, _: u32) -> u32 {
    // SAFETY: SetLastError only changes the calling thread's error slot.
    unsafe { SetLastError(5) };
    u32::MAX
}

static CLOSED: AtomicUsize = AtomicUsize::new(0);
unsafe extern "system" fn counted_close(handle: Handle) -> i32 {
    CLOSED.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the injected boundary receives the Signal's sole owned handle.
    unsafe { CloseHandle(handle) }
}

#[test]
fn signal_closes_its_handle_once_after_the_last_owner() {
    CLOSED.store(0, Ordering::SeqCst);
    let signal = Signal::create(Api {
        close: counted_close,
        ..API
    })
    .unwrap();
    let owner = signal.clone();
    drop(signal);
    assert_eq!(CLOSED.load(Ordering::SeqCst), 0);
    drop(owner);
    assert_eq!(CLOSED.load(Ordering::SeqCst), 1);
}

pub(crate) fn failed_create() -> Result<Arc<Signal>, u32> {
    Signal::create(Api {
        create: no_event,
        ..API
    })
}

pub(crate) fn failed_submit() -> Result<Arc<Signal>, u32> {
    Signal::create(Api {
        submit: no_submission,
        ..API
    })
}

std::thread_local! {
    static SUBMISSIONS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

unsafe extern "system" fn one_submission(
    callback: Callback,
    context: Handle,
    environment: Handle,
) -> i32 {
    if SUBMISSIONS.with(|count| {
        let previous = count.get();
        count.set(previous + 1);
        previous == 0
    }) {
        // SAFETY: forward the unchanged callback and payload to the real pool.
        unsafe { TrySubmitThreadpoolCallback(callback, context, environment) }
    } else {
        // SAFETY: this injected failure never transfers ownership to a worker.
        unsafe { no_submission(callback, context, environment) }
    }
}

pub(crate) fn fail_after_first_submission() -> Result<Arc<Signal>, u32> {
    SUBMISSIONS.with(|count| count.set(0));
    Signal::create(Api {
        submit: one_submission,
        ..API
    })
}

pub(crate) fn failed_wait() -> Result<Arc<Signal>, u32> {
    Signal::create(Api {
        wait: no_wait,
        ..API
    })
}

pub(crate) fn signal_with_wait(
    wait: unsafe extern "system" fn(Handle, u32) -> u32,
) -> Result<Arc<Signal>, u32> {
    Signal::create(Api { wait, ..API })
}

pub(crate) fn wait_for_worker(signal: &Signal) {
    // SAFETY: the referenced Signal keeps its valid event handle open.
    assert_eq!(unsafe { WaitForSingleObject(signal.handle, 5_000) }, 0);
}

pub(crate) fn publish_wake(signal: &Signal) {
    // SAFETY: simulate a worker's publication on the Signal's live event.
    assert_ne!(unsafe { SetEvent(signal.handle) }, 0);
}

#[test]
fn event_creation_and_wait_failures_preserve_os_errors() {
    assert_eq!(failed_create().err(), Some(5));
    let signal = Signal::new().unwrap();
    assert_eq!(signal.wait(0), Ok(false));
    assert_eq!(failed_wait().unwrap().wait(0), Err(5));
}

struct Capture(Arc<AtomicUsize>);

impl Drop for Capture {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
        // SAFETY: test cleanup changes only this thread's error slot.
        unsafe { SetLastError(77) };
    }
}

#[test]
fn rejected_submission_releases_captures_and_preserves_error_before_drop() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let capture = Capture(dropped.clone());
    let signal = failed_submit().unwrap();
    let directory: Vec<u16> = std::env::temp_dir()
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect();
    let result = Work::submit(signal.clone(), move || {
        drop(capture);
        fs_win::stat(&directory, false)
    });
    assert_eq!(result.err(), Some(5));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(Arc::strong_count(&signal), 1);
}

#[test]
fn filesystem_work_runs_off_thread_and_result_is_taken_once() {
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    let directory = std::env::temp_dir().join(format!(
        "nativeboat-worker-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("stat-中🚀.txt");
    std::fs::write(&path, b"nativeboat").unwrap();
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let main_thread = std::thread::current().id();
    let (thread, observed_thread) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let signal = Signal::new().unwrap();
    let mut work = Work::submit(signal.clone(), move || {
        wait.recv().unwrap();
        thread.send(std::thread::current().id()).unwrap();
        fs_win::stat(&wide, false)
    })
    .unwrap();
    assert!(work.take().is_none());
    assert_eq!(signal.wait(0), Ok(false));
    release.send(()).unwrap();
    assert_eq!(signal.wait(5_000), Ok(true));
    let result = work.take().unwrap();
    let worker_thread = observed_thread
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert_ne!(worker_thread, main_thread);
    assert_eq!(result.unwrap().size, 10);
    assert!(work.take().is_none());
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

struct ResultDrop(mpsc::Sender<()>);

impl Drop for ResultDrop {
    fn drop(&mut self) {
        self.0.send(()).unwrap();
    }
}

#[test]
fn abandoning_pending_work_keeps_worker_state_alive_until_completion() {
    let (release, wait) = mpsc::channel();
    let (dropped, observed) = mpsc::channel();
    let signal = Signal::new().unwrap();
    let work = Work::submit(signal.clone(), move || {
        wait.recv().unwrap();
        ResultDrop(dropped)
    })
    .unwrap();
    drop(work);
    assert!(observed.try_recv().is_err());
    release.send(()).unwrap();
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(Arc::strong_count(&signal), 1);
}
