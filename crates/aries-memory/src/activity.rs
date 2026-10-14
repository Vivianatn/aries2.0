use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// Compte les réponses en cours, pour que les tâches de fond laissent le CPU au chat.
#[derive(Clone, Default)]
pub struct ActivityGauge(Arc<AtomicUsize>);

impl ActivityGauge {
    pub fn begin(&self) -> ActivityGuard {
        self.0.fetch_add(1, Ordering::SeqCst);
        ActivityGuard(self.0.clone())
    }

    pub fn is_idle(&self) -> bool {
        self.0.load(Ordering::SeqCst) == 0
    }
}

pub struct ActivityGuard(Arc<AtomicUsize>);

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
