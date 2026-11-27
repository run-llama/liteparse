//! The document time budget during layout: layout code calls [`check`] at each
//! paragraph, which unwinds with [`Expired`] once the budget set by
//! [`start`] has run out; the driver turns that unwind into a timeout.

use std::cell::Cell;
use std::time::{Duration, Instant};

thread_local! {
    static DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The panic payload of an expired budget.
pub struct Expired;

/// Sets the budget for layout on this thread (`None` clears it).
pub fn start(budget: Option<Duration>) {
    DEADLINE.with(|d| d.set(budget.map(|b| Instant::now() + b)));
}

/// Unwinds with [`Expired`] when this thread's budget has run out.
pub fn check() {
    if let Some(at) = DEADLINE.with(|d| d.get())
        && Instant::now() > at
    {
        std::panic::resume_unwind(Box::new(Expired));
    }
}

/// Whether a caught panic payload is an expired budget.
pub fn expired(p: &Box<dyn std::any::Any + Send>) -> bool {
    p.is::<Expired>()
}

#[cfg(test)]
mod tests;
