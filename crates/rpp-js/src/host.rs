//! Host functions reachable from JavaScript while a call is running.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

use deno_core::{op2, OpState};
use deno_error::JsErrorBox;

use crate::model::Host;

/// The host of the call in progress.
struct Slot(Option<NonNull<dyn Host>>);

/// Bytes crossing the boundary: the call's input, or the last host reply.
struct Pending(Option<Vec<u8>>);

pub(crate) fn initialize(state: &mut OpState) {
    state.put(Slot(None));
    state.put(Pending(None));
}

pub(crate) fn set_pending(state: &mut OpState, bytes: Option<Vec<u8>>) {
    state.put(Pending(bytes));
}

/// Makes a host reachable from ops until dropped, including during unwind.
pub(crate) struct Installed<'a> {
    state: Rc<RefCell<OpState>>,
    _host: PhantomData<&'a mut dyn Host>,
}

impl<'a> Installed<'a> {
    pub(crate) fn new(state: Rc<RefCell<OpState>>, host: &'a mut dyn Host) -> Self {
        let pointer: NonNull<dyn Host + 'a> = NonNull::from(host);
        // SAFETY: only the trait object's lifetime bound changes; the layout is identical.
        // The pointer is dereferenced only by `op_rpp_host` while `self` is alive, and
        // `Drop` clears the slot before the borrow of `host` ends.
        let pointer: NonNull<dyn Host + 'static> = unsafe { std::mem::transmute(pointer) };
        state.borrow_mut().borrow_mut::<Slot>().0 = Some(pointer);
        Self {
            state,
            _host: PhantomData,
        }
    }
}

impl Drop for Installed<'_> {
    fn drop(&mut self) {
        self.state.borrow_mut().borrow_mut::<Slot>().0 = None;
    }
}

#[op2]
#[string]
fn op_rpp_host(
    state: &mut OpState,
    #[string] name: String,
    #[string] value: String,
    has_bytes: bool,
    #[buffer] bytes: &[u8],
) -> Result<String, JsErrorBox> {
    let Some(mut host) = state.borrow::<Slot>().0 else {
        return Err(JsErrorBox::generic(
            "host functions are unavailable during module evaluation",
        ));
    };
    let value = serde_json::from_str(&value).map_err(JsErrorBox::from_err)?;
    let bytes = has_bytes.then(|| bytes.to_vec());
    // SAFETY: `Installed` keeps the pointee borrowed for the whole call and clears the slot
    // on drop. Ops run on the call's thread and the host cannot re-enter JavaScript, so
    // this is the only live reference.
    let reply = unsafe { host.as_mut() }
        .call(&name, value, bytes)
        .map_err(JsErrorBox::generic)?;
    state.put(Pending(reply.bytes));
    serde_json::to_string(&reply.value).map_err(JsErrorBox::from_err)
}

/// Length of the pending bytes, or -1 when there are none.
#[op2(fast)]
#[smi]
fn op_rpp_pending_length(state: &mut OpState) -> i32 {
    state
        .borrow::<Pending>()
        .0
        .as_ref()
        .map_or(-1, |bytes| i32::try_from(bytes.len()).unwrap_or(i32::MAX))
}

/// Moves the pending bytes into `out`, which JavaScript sizes with `op_rpp_pending_length`.
#[op2(fast)]
fn op_rpp_take_pending(state: &mut OpState, #[buffer] out: &mut [u8]) {
    if let Some(bytes) = state.borrow_mut::<Pending>().0.take() {
        let length = out.len().min(bytes.len());
        out[..length].copy_from_slice(&bytes[..length]);
    }
}

deno_core::extension!(
    rpp_host,
    ops = [op_rpp_host, op_rpp_pending_length, op_rpp_take_pending]
);
