use std::task::Context;
use std::task::Poll;
use std::rc::Rc;
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;

use futures::task::AtomicWaker;

#[derive(Copy, Clone, Debug)]
pub struct Cancelled;

#[derive(Clone)]
pub struct CancelHandle {
	inner: Rc<CancelInner>,
}

pub struct CancelInner {
	cancelled: Cell<bool>,
	waker: AtomicWaker,
}

#[derive(Clone)]
pub struct Cancelable<Fut> {
	inner: Fut,
	cancel: CancelHandle,
}

impl CancelHandle {
	fn new() -> Self {
		Self {
			inner: Rc::new(CancelInner {
				cancelled: Cell::new(false),
				waker: AtomicWaker::new(),
			}),
		}
	}

	pub fn cancel(&self) {
		self.inner.cancelled.set(true);
		self.inner.waker.wake();
	}
}

impl<Fut: Future> Cancelable<Fut> {
	fn new(inner: Fut, cancel: CancelHandle) -> Self {
		Self {
			inner,
			cancel,
		}
	}

	pub fn inner(&self) -> &Fut {
		&self.inner
	}

	pub fn inner_mut(&mut self) -> &mut Fut where Fut: Unpin {
		&mut self.inner
	}

	pub fn inner_pin_mut(self: Pin<&mut Self>) -> Pin<&mut Fut> {
		unsafe { self.map_unchecked_mut(|x| &mut x.inner) }
	}

	pub fn into_inner(self) -> Fut where Fut: Unpin {
		self.inner
	}
}

impl<Fut: Future> Future for Cancelable<Fut> {
	type Output = Result<Fut::Output, Cancelled>;

	fn poll(mut self: Pin<&mut Self>, context: &mut Context) -> Poll<Self::Output> {
		if let Poll::Ready(x) = self.as_mut().inner_pin_mut().poll(context) {
			return Poll::Ready(Ok(x));
		}

		self.cancel.inner.waker.register(context.waker());
		if self.cancel.inner.cancelled.get() {
			Poll::Ready(Err(Cancelled))
		} else {
			Poll::Pending
		}
	}
}

pub fn cancelable<Fut: Future>(future: Fut) -> (Cancelable<Fut>, CancelHandle) {
	let cancel = CancelHandle::new();
	let cancelable = Cancelable::new(future, cancel.clone());
	(cancelable, cancel)
}
