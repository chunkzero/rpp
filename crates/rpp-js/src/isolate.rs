//! Parks each runtime's isolate while it is idle so many runtimes can share a thread.

use crate::deadline::Deadline;
use crate::error::Result;
use crate::model::{Call, Cancellation, Clock, Host, Limits, Log, Output};
use crate::runtime::State;

/// A runtime is exited while parked. All access and destruction enter it on its owning
/// thread. `State` (and therefore this wrapper) is neither `Send` nor `Sync`.
pub(crate) struct Isolate(Option<State>);

impl Isolate {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn load(
        executor: &tokio::runtime::Runtime,
        deadline: &Deadline,
        name: &str,
        code: &str,
        source_map: &str,
        limits: Limits,
        clock: Clock,
        cancellation: &Cancellation,
    ) -> Result<(Self, Vec<Log>)> {
        let _executor = executor.enter();
        let mut state = State::new(limits, name, source_map);
        // SAFETY: JsRuntime creation enters this isolate. No handle scopes remain;
        // exiting balances creation and restores the previously current isolate.
        unsafe { state.runtime.v8_isolate().exit() };
        let mut isolate = Self(Some(state));
        let logs = isolate.with(|state| {
            state.initialize_on(executor, deadline, code, limits, clock, cancellation)
        })?;
        Ok((isolate, logs))
    }

    pub(crate) fn call(
        &mut self,
        executor: &tokio::runtime::Runtime,
        deadline: &Deadline,
        call: Call<'_>,
        host: &mut dyn Host,
        limits: Limits,
        cancellation: &Cancellation,
    ) -> Result<Output> {
        let _executor = executor.enter();
        self.with(|state| state.call(executor, deadline, call, host, limits, cancellation))
    }

    pub(crate) fn is_terminated(&self) -> bool {
        self.0.as_ref().expect("live runtime").terminated
    }

    fn with<T>(&mut self, run: impl FnOnce(&mut State) -> T) -> T {
        let state = self.0.as_mut().expect("live runtime");
        // SAFETY: The wrapper cannot cross threads, and exclusive access prevents
        // concurrent use. `Entered` restores the previous isolate even during unwind.
        unsafe { state.runtime.v8_isolate().enter() };
        let entered = Entered(state);
        run(entered.0)
    }
}

struct Entered<'a>(&'a mut State);

impl Drop for Entered<'_> {
    fn drop(&mut self) {
        // SAFETY: `with` entered this isolate on this thread; its callback has returned
        // or unwound, dropping all scopes before this matching exit.
        unsafe { self.0.runtime.v8_isolate().exit() };
    }
}

impl Drop for Isolate {
    fn drop(&mut self) {
        if let Some(mut state) = self.0.take() {
            // SAFETY: This parked isolate remains owned by this thread. `State` drops its
            // persistent handles before `JsRuntime`, whose isolate destructor performs the
            // matching exit and disposal while this isolate is current.
            unsafe { state.runtime.v8_isolate().enter() };
            drop(state);
        }
    }
}
