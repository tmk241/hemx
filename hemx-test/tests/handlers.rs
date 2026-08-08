use hemx_core::{Effect, GeneratedTarget, ResourceId, ResourceKind, Slot};
use std::future::Future;
use std::task::{Context, Poll, Waker};

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

#[derive(Clone, Copy)]
struct CountTarget;

impl GeneratedTarget for CountTarget {
    fn __hemx_resource_id(self) -> ResourceId {
        ResourceId::new(ResourceKind::Slot, 1)
    }
}

fn sync_handler(value: u32) -> Effect {
    Slot::<u32>::new(1).text(value)
}

async fn async_handler(value: u32) -> Effect {
    std::future::ready(()).await;
    Slot::<u32>::new(1).text(value)
}

#[derive(Debug, Eq, PartialEq)]
struct Rejected {
    value: u32,
}

fn fallible_handler(value: u32) -> Result<Effect, Rejected> {
    if value == 0 {
        Err(Rejected { value })
    } else {
        Ok(sync_handler(value))
    }
}

async fn fallible_async_handler(value: u32) -> Result<Effect, Rejected> {
    std::future::ready(()).await;
    if value == 0 {
        Err(Rejected { value })
    } else {
        Ok(sync_handler(value))
    }
}

#[test]
fn runs_sync_and_async_handlers_into_the_same_effect_inspector() {
    let sync = hemx_test::run(sync_handler, 41);
    assert!(sync.updates_text_containing(CountTarget, "41"));

    let asynchronous = block_on(hemx_test::run_async(async_handler, 42));
    assert!(asynchronous.updates_text_containing(CountTarget, "42"));
}

#[test]
fn inspects_successful_fallible_handlers() {
    let sync = hemx_test::run_result(fallible_handler, 41).unwrap();
    assert!(sync.updates_text_containing(CountTarget, "41"));

    let asynchronous = block_on(hemx_test::run_async_result(fallible_async_handler, 42)).unwrap();
    assert!(asynchronous.updates_text_containing(CountTarget, "42"));
}

#[test]
fn preserves_concrete_sync_and_async_handler_errors() {
    let sync = hemx_test::run_result(fallible_handler, 0).unwrap_err();
    assert_eq!(sync, Rejected { value: 0 });

    let asynchronous =
        block_on(hemx_test::run_async_result(fallible_async_handler, 0)).unwrap_err();
    assert_eq!(asynchronous, Rejected { value: 0 });
}
