//! A count of the work the views do, for tests that check that work grows
//! in proportion to the input.
//!
//! Every access to a node of a tree and every step of a loop over an
//! in-memory list in the views calls [`tick`]. In a test build the calls
//! add up in a counter of the thread; in other builds they cost nothing.

#[cfg(test)]
thread_local! {
    static WORK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Counts `n` steps of work.
#[inline(always)]
pub(crate) fn tick(n: u64) {
    #[cfg(test)]
    WORK.with(|w| w.set(w.get().saturating_add(n)));
    #[cfg(not(test))]
    let _ = n;
}

/// The work counted on this thread so far.
#[cfg(test)]
pub(crate) fn work() -> u64 {
    WORK.with(|w| w.get())
}
