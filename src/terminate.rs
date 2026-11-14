//! Shared child-termination escalation policy.
//!
//! The policy is runtime-independent: callers inject the sleep function. The
//! Rust session uses [`tokio::time::sleep`]; the PyO3 binding uses
//! `spawn_blocking(std::thread::sleep)` because tokio timers can fail to fire
//! through the Python bridge on macOS (see `docs/TESTS.md`).
//!
//! Both SIGTERM and SIGKILL are best-effort. An already-exited child, or a
//! platform that rejects a signal, is not an error: the policy only bounds how
//! long it waits for the cached reaper state to report the exit.

use crate::platform::ArcChildBackend;
use std::time::{Duration, Instant};

/// Poll interval for the cached child state while waiting for an exit.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Hard bound for the post-SIGKILL reap wait.
const KILL_REAP_TIMEOUT: Duration = Duration::from_secs(2);

/// SIGTERM, wait up to `grace`, SIGKILL, then wait up to two seconds for the
/// reaper/watcher to publish the exit.
///
/// The caller supplies `sleep` so the same policy can run on a tokio runtime
/// or through PyO3 without relying on tokio timers in the latter.
pub(crate) async fn terminate_child<S, Fut>(child: &ArcChildBackend, grace: Duration, mut sleep: S)
where
    S: FnMut(Duration) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let _ = child.signal(15);

    let deadline = Instant::now() + grace;
    while child.is_running() && Instant::now() < deadline {
        sleep(POLL_INTERVAL).await;
    }

    if !child.is_running() {
        return;
    }

    let _ = child.kill();
    let kill_deadline = Instant::now() + KILL_REAP_TIMEOUT;
    while child.is_running() && Instant::now() < kill_deadline {
        sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{ChildBackend, ChildKiller, ExitStatus};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeState {
        running: bool,
        exit_on_sigterm: bool,
        signals: Vec<i32>,
        kills: usize,
    }

    struct FakeChild {
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeChild {
        fn new(running: bool, exit_on_sigterm: bool) -> Self {
            Self {
                state: Arc::new(Mutex::new(FakeState {
                    running,
                    exit_on_sigterm,
                    ..FakeState::default()
                })),
            }
        }

        fn state(&self) -> Arc<Mutex<FakeState>> {
            Arc::clone(&self.state)
        }
    }

    struct FakeKiller {
        state: Arc<Mutex<FakeState>>,
    }

    fn kill(state: &Arc<Mutex<FakeState>>) {
        let mut state = state.lock().unwrap();
        state.kills += 1;
        state.running = false;
    }

    impl ChildKiller for FakeKiller {
        fn kill(&self) -> crate::errors::PtyResult<()> {
            kill(&self.state);
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
            Box::new(Self {
                state: Arc::clone(&self.state),
            })
        }
    }

    impl ChildKiller for FakeChild {
        fn kill(&self) -> crate::errors::PtyResult<()> {
            kill(&self.state);
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
            Box::new(FakeKiller {
                state: Arc::clone(&self.state),
            })
        }
    }

    #[async_trait::async_trait]
    impl ChildBackend for FakeChild {
        fn pid(&self) -> u32 {
            1
        }

        fn is_running(&self) -> bool {
            self.state.lock().unwrap().running
        }

        async fn wait(&self) -> Option<ExitStatus> {
            None
        }

        fn signal(&self, sig: i32) -> crate::errors::PtyResult<()> {
            let mut state = self.state.lock().unwrap();
            state.signals.push(sig);
            if sig == 15 && state.exit_on_sigterm {
                state.running = false;
            }
            Ok(())
        }
    }

    fn no_sleep() -> impl FnMut(Duration) -> std::future::Ready<()> {
        |_| std::future::ready(())
    }

    fn fake(running: bool, exit_on_sigterm: bool) -> (ArcChildBackend, Arc<Mutex<FakeState>>) {
        let child = FakeChild::new(running, exit_on_sigterm);
        let state = child.state();
        (Arc::new(child), state)
    }

    #[tokio::test]
    async fn already_exited_child_is_not_killed() {
        let (child, state) = fake(false, false);
        terminate_child(&child, Duration::from_secs(1), no_sleep()).await;

        let state = state.lock().unwrap();
        assert_eq!(state.signals, vec![15]);
        assert_eq!(state.kills, 0);
    }

    #[tokio::test]
    async fn graceful_exit_skips_sigkill() {
        let (child, state) = fake(true, true);
        terminate_child(&child, Duration::from_secs(1), no_sleep()).await;

        let state = state.lock().unwrap();
        assert_eq!(state.kills, 0);
        assert!(!state.running);
    }

    #[tokio::test]
    async fn sigterm_ignoring_child_escalates_to_sigkill() {
        let (child, state) = fake(true, false);
        terminate_child(&child, Duration::ZERO, no_sleep()).await;

        let state = state.lock().unwrap();
        assert_eq!(state.signals, vec![15]);
        assert_eq!(state.kills, 1);
        assert!(!state.running);
    }

    #[tokio::test]
    async fn repeated_calls_reuse_the_shared_policy() {
        let (child, state) = fake(true, false);
        terminate_child(&child, Duration::ZERO, no_sleep()).await;
        terminate_child(&child, Duration::ZERO, no_sleep()).await;

        let state = state.lock().unwrap();
        assert_eq!(state.signals, vec![15, 15]);
        assert_eq!(state.kills, 1);
    }

    #[tokio::test]
    async fn injected_sleep_drives_the_grace_wait() {
        let (child, state) = fake(true, false);
        let sleep_state = Arc::clone(&state);
        let mut ticks = 0;
        let sleep = move |_| {
            ticks += 1;
            if ticks == 2 {
                sleep_state.lock().unwrap().running = false;
            }
            std::future::ready(())
        };

        terminate_child(&child, Duration::from_secs(1), sleep).await;

        let state = state.lock().unwrap();
        assert_eq!(state.kills, 0);
        assert!(!state.running);
    }
}
