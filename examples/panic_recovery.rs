//! Exercise worker recovery in a real executable, including release builds.
//! `cargo test --release` alone cannot verify the production panic strategy:
//! the Rust test harness uses unwinding even when the release profile aborts.

use apexstore::infra::panic_recovery::PanicRecovery;
use parking_lot::Mutex;
use std::io;
use std::sync::Arc;

fn main() -> io::Result<()> {
    let recovery = PanicRecovery::new();
    let callbacks = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&callbacks);
    recovery.on_panic(Box::new(move |info| {
        captured.lock().push(info.clone());
    }));

    // The default panic hook logs this intentional worker failure. A healthy
    // process must reach the assertions below and exit successfully.
    let failed = recovery
        .spawn_protected::<_, ()>(Some("recoverable_worker"), || {
            panic!("intentional worker recovery regression");
        })
        .join()
        .map_err(|_| io::Error::other("worker panic escaped its recovery guard"))?;
    assert_eq!(failed, None);
    let history = recovery.recent_panics();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].reason, "intentional worker recovery regression");
    assert_eq!(
        history[0].thread_name.as_deref(),
        Some("recoverable_worker")
    );
    let callbacks = callbacks.lock();
    assert_eq!(callbacks.len(), 1);
    assert_eq!(callbacks[0].reason, history[0].reason);
    assert_eq!(callbacks[0].thread_name, history[0].thread_name);
    drop(callbacks);

    let healthy = recovery
        .spawn_protected(Some("healthy_worker"), || 42)
        .join()
        .map_err(|_| io::Error::other("healthy worker unexpectedly panicked"))?;
    assert_eq!(healthy, Some(42));
    assert_eq!(recovery.recent_panics().len(), 1);
    recovery.clear_history();
    assert!(recovery.recent_panics().is_empty());
    Ok(())
}
