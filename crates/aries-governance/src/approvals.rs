//! Validations humaines en attente pour les actions `danger`.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::sync::oneshot;

use crate::Decision;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<bool>>>>;

#[derive(Default)]
pub struct Approvals {
    pending: Pending,
    next_id: AtomicU64,
}

impl Approvals {
    /// Ouvre une demande ; son identifiant est transmis à l'utilisateur.
    pub fn request(&self) -> PendingApproval {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (sender, receiver) = oneshot::channel();
        lock(&self.pending).insert(id, sender);
        PendingApproval {
            id,
            receiver,
            pending: self.pending.clone(),
        }
    }

    /// Transmet la réponse de l'utilisateur. `false` si la demande n'existe plus.
    pub fn resolve(&self, id: u64, approved: bool) -> bool {
        lock(&self.pending)
            .remove(&id)
            .is_some_and(|sender| sender.send(approved).is_ok())
    }
}

pub struct PendingApproval {
    pub id: u64,
    receiver: oneshot::Receiver<bool>,
    pending: Pending,
}

impl PendingApproval {
    pub async fn wait(self, timeout: Duration) -> Decision {
        let decision = match tokio::time::timeout(timeout, self.receiver).await {
            Ok(Ok(true)) => Decision::Approved,
            Ok(Ok(false)) => Decision::Rejected,
            Ok(Err(_)) | Err(_) => Decision::Expired,
        };
        lock(&self.pending).remove(&self.id);
        decision
    }
}

/// La table reste utilisable même si une tâche a paniqué en la tenant.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolves_and_expires() {
        let approvals = Approvals::default();

        let pending = approvals.request();
        assert!(approvals.resolve(pending.id, true));
        assert_eq!(
            pending.wait(Duration::from_secs(1)).await,
            Decision::Approved
        );

        let pending = approvals.request();
        let id = pending.id;
        assert_eq!(
            pending.wait(Duration::from_millis(10)).await,
            Decision::Expired
        );
        assert!(!approvals.resolve(id, true));
    }
}
