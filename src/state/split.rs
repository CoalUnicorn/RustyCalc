//! Zero-cost reactive signal-pair primitive.

use leptos::prelude::*;

/// Zero-cost wrapper around a Leptos `(ReadSignal, WriteSignal)` pair.
pub struct Split<T: Clone + Send + Sync + 'static>(ReadSignal<T>, WriteSignal<T>);

// Manual impls: ReadSignal<T>/WriteSignal<T> are always Copy (arena IDs),
// so Split<T> is Copy for any T - even non-Copy types like String or Vec.
// #[derive(Copy)] would incorrectly add a T: Copy bound.
impl<T: Clone + Send + Sync + 'static> Clone for Split<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Clone + Send + Sync + 'static> Copy for Split<T> {}

impl<T: Clone + Send + Sync + 'static> Split<T> {
    pub fn new(initial: T) -> Self {
        let (r, w) = signal(initial);
        Self(r, w)
    }

    pub fn get(&self) -> T {
        self.0.get()
    }

    pub fn get_untracked(&self) -> T {
        self.0.get_untracked()
    }

    #[allow(dead_code)]
    /// Reads the inner value via a closure. Stable primitive API;
    /// kept for potential external consumers that prefer the `with`
    /// pattern over `get()`.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.0.with(f)
    }

    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.0.with_untracked(f)
    }

    pub fn set(&self, v: T) {
        self.1.set(v);
    }

    pub fn update(&self, f: impl FnOnce(&mut T)) {
        self.1.update(f);
    }

    #[allow(dead_code)]
    /// Returns the inner `ReadSignal`. Stable primitive API;
    /// kept for potential external consumers that need raw signal handles.
    pub fn read(&self) -> ReadSignal<T> {
        self.0
    }

    #[allow(dead_code)]
    /// Returns the inner `WriteSignal`. Stable primitive API;
    /// kept for potential external consumers that need raw signal handles.
    pub fn write(&self) -> WriteSignal<T> {
        self.1
    }
}
