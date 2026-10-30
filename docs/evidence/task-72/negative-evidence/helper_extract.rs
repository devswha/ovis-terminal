/// How long a joined thread may linger in `/proc/self/task`: `pthread_join`
/// returns once the kernel clears the child's tid, slightly before the kernel
/// removes the task entry.
const TEARDOWN_DEADLINE: Duration = Duration::from_secs(2);

/// What waiting for a thread family to disappear found. `Unavailable` is a
/// third state, never folded into `Gone`: where the OS offers no thread
/// listing the probe cannot know.
#[derive(Debug, PartialEq, Eq)]
enum Teardown {
    Gone,
    Leftover(Vec<String>),
    Unavailable { reason: io::ErrorKind },
}

/// Names of this test process's threads that start with `prefix`; on Linux the
/// kernel exposes each thread's name under `/proc/self/task`. The error is the
/// kind of the failed listing (`NotFound` where `/proc` does not exist).
fn thread_names(prefix: &str) -> Result<Vec<String>, io::ErrorKind> {
    let tasks = std::fs::read_dir("/proc/self/task").map_err(|error| error.kind())?;
    Ok(tasks
        .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
        .map(|name| name.trim_end().to_owned())
        .filter(|name| name.starts_with(prefix))
        .collect())
}

/// Waits until at most `baseline` threads named `prefix…` remain; a thread
/// that never exits is reported by name once `deadline` passes.
fn await_threads_at_most(prefix: &str, baseline: usize, deadline: Duration) -> Teardown {
    let started = Instant::now();
    loop {
        match thread_names(prefix) {
            Err(reason) => return Teardown::Unavailable { reason },
            Ok(names) if names.len() <= baseline => return Teardown::Gone,
            Ok(names) if started.elapsed() >= deadline => return Teardown::Leftover(names),
            Ok(_) => thread::yield_now(),
        }
    }
}

/// The outcome this OS must produce when `linux` is what Linux produces.
fn on_this_os(linux: Teardown) -> Teardown {
    if cfg!(target_os = "linux") {
        linux
    } else {
        Teardown::Unavailable {
            reason: io::ErrorKind::NotFound,
        }
    }
}

