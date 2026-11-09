//! Turn SIGTERM/SIGINT/SIGHUP into a normal event-loop exit, so progress is
//! flushed and the launcher's restore step runs (spec §7, §21.3).
//!
//! The signals are blocked in every thread and received by one dedicated
//! thread with `sigwait` — no polling and no async-signal-safety pitfalls.

/// Must be called before any other thread is spawned.
pub fn block_termination_signals() -> libc::sigset_t {
    // SAFETY: sigset manipulation on a local, then pthread_sigmask for the
    // current (only) thread; spawned threads inherit the mask.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::sigaddset(&mut set, libc::SIGHUP);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    }
}

/// Spawn the waiter. On the first signal, ask the UI loop to quit.
pub fn spawn_quit_on_signal(set: libc::sigset_t) {
    let _ = std::thread::Builder::new()
        .name("signal-wait".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            let mut sig: libc::c_int = 0;
            // SAFETY: `set` is a valid, initialised sigset.
            let rc = unsafe { libc::sigwait(&set, &mut sig) };
            if rc == 0 {
                eprintln!("eink-reader: signal {sig}, exiting cleanly");
                let _ = slint::invoke_from_event_loop(|| {
                    let _ = slint::quit_event_loop();
                });
            }
        });
}
