use ahash::AHasher;
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::rules::traits::StreamingRule;

use super::state::UserState;
use super::user::UserActor;

/// Number of lock stripes for the actor pool.
/// Must be a power of 2 for fast modulo via bitwise AND.
/// Stripes reduce lock contention within a single process (not distributed partitions).
const NUM_STRIPES: usize = 64;

/// Actor pool managing user actors with striped locking.
///
/// Users are distributed across lock stripes based on their ID hash.
/// This minimizes lock contention for concurrent requests within a single process.
/// Note: Stripes are for reducing contention, not for distributed partitioning.
pub struct ActorPool {
    /// Striped actor storage (each stripe has its own lock)
    stripes: Vec<RwLock<HashMap<String, Arc<Mutex<UserActor>>>>>,

    /// Streaming rules shared across all actors
    streaming_rules: Arc<Vec<Arc<dyn StreamingRule>>>,
}

impl ActorPool {
    /// Create a new actor pool.
    pub fn new(streaming_rules: Vec<Arc<dyn StreamingRule>>) -> Self {
        let stripes = (0..NUM_STRIPES)
            .map(|_| RwLock::new(HashMap::new()))
            .collect();

        ActorPool {
            stripes,
            streaming_rules: Arc::new(streaming_rules),
        }
    }

    /// Get or create an actor for the given user.
    ///
    /// Returns a mutex-guarded actor that can be locked for exclusive access.
    pub fn get_or_create(&self, user_id: &str) -> Arc<Mutex<UserActor>> {
        let stripe_idx = self.stripe_index(user_id);
        let stripe = &self.stripes[stripe_idx];

        // Fast path: check if actor exists with read lock
        {
            let read_guard = stripe.read();
            if let Some(actor) = read_guard.get(user_id) {
                return actor.clone();
            }
        }

        // Slow path: create actor with write lock
        let mut write_guard = stripe.write();

        // Double-check after acquiring write lock
        if let Some(actor) = write_guard.get(user_id) {
            return actor.clone();
        }

        // Create new actor
        let actor = Arc::new(Mutex::new(UserActor::new(
            user_id.to_string(),
            self.streaming_rules.clone(),
        )));

        write_guard.insert(user_id.to_string(), actor.clone());
        actor
    }

    /// Get an existing actor without creating.
    pub fn get(&self, user_id: &str) -> Option<Arc<Mutex<UserActor>>> {
        let stripe_idx = self.stripe_index(user_id);
        let stripe = &self.stripes[stripe_idx];

        let read_guard = stripe.read();
        read_guard.get(user_id).cloned()
    }

    /// Insert an actor with existing state (for recovery).
    pub fn insert_with_state(&self, state: UserState) {
        let user_id = state.user_id.clone();
        let stripe_idx = self.stripe_index(&user_id);
        let stripe = &self.stripes[stripe_idx];

        let actor = Arc::new(Mutex::new(UserActor::with_state(
            state,
            self.streaming_rules.clone(),
        )));

        let mut write_guard = stripe.write();
        write_guard.insert(user_id, actor);
    }

    /// Update streaming rules for all actors.
    ///
    /// This is called during hot reload to update rules without
    /// losing user state.
    pub fn update_rules(&self, rules: Vec<Arc<dyn StreamingRule>>) {
        let rules = Arc::new(rules);

        for stripe in &self.stripes {
            let read_guard = stripe.read();
            for actor in read_guard.values() {
                actor.lock().update_rules(rules.clone());
            }
        }
    }

    /// Remove idle actors to free memory.
    ///
    /// Returns the number of actors evicted.
    pub fn evict_idle(&self, idle_threshold_secs: i64) -> usize {
        let mut evicted = 0;

        for stripe in &self.stripes {
            let mut write_guard = stripe.write();
            let before = write_guard.len();

            write_guard.retain(|_, actor| {
                !actor.lock().is_idle(idle_threshold_secs)
            });

            evicted += before - write_guard.len();
        }

        evicted
    }

    /// Get the total number of actors.
    pub fn actor_count(&self) -> usize {
        self.stripes
            .iter()
            .map(|s| s.read().len())
            .sum()
    }

    /// Get statistics about the pool.
    pub fn stats(&self) -> PoolStats {
        let mut total_actors = 0;
        let mut total_entries = 0;
        let mut stripe_sizes = Vec::with_capacity(NUM_STRIPES);

        for stripe in &self.stripes {
            let read_guard = stripe.read();
            let stripe_size = read_guard.len();
            stripe_sizes.push(stripe_size);
            total_actors += stripe_size;

            for actor in read_guard.values() {
                total_entries += actor.lock().entry_count();
            }
        }

        PoolStats {
            total_actors,
            total_entries,
            stripe_sizes,
        }
    }

    /// Compute the stripe index for a user ID.
    #[inline]
    fn stripe_index(&self, user_id: &str) -> usize {
        let mut hasher = AHasher::default();
        user_id.hash(&mut hasher);
        (hasher.finish() as usize) & (NUM_STRIPES - 1)
    }
}

/// Statistics about the actor pool.
#[derive(Debug)]
pub struct PoolStats {
    /// Total number of actors
    pub total_actors: usize,
    /// Total number of transaction entries across all actors
    pub total_entries: usize,
    /// Number of actors per stripe
    pub stripe_sizes: Vec<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::streaming::DailyVolumeRule;
    use crate::domain::Decision;
    use rust_decimal::Decimal;

    fn test_rules() -> Vec<Arc<dyn StreamingRule>> {
        vec![Arc::new(DailyVolumeRule::new(
            "R4".to_string(),
            Decision::HoldAuto,
            Decimal::new(50000, 0),
        ))]
    }

    #[test]
    fn test_get_or_create() {
        let pool = ActorPool::new(test_rules());

        let actor1 = pool.get_or_create("user1");
        let actor2 = pool.get_or_create("user1");

        // Same user should return same actor
        assert!(Arc::ptr_eq(&actor1, &actor2));

        let actor3 = pool.get_or_create("user2");
        // Different user should return different actor
        assert!(!Arc::ptr_eq(&actor1, &actor3));

        assert_eq!(pool.actor_count(), 2);
    }

    #[test]
    fn test_get_nonexistent() {
        let pool = ActorPool::new(test_rules());

        assert!(pool.get("nonexistent").is_none());

        pool.get_or_create("user1");
        assert!(pool.get("user1").is_some());
    }

    #[test]
    fn test_insert_with_state() {
        let pool = ActorPool::new(test_rules());

        let state = UserState::new("recovered_user".to_string());
        pool.insert_with_state(state);

        assert!(pool.get("recovered_user").is_some());
    }

    #[test]
    fn test_stats() {
        let pool = ActorPool::new(test_rules());

        pool.get_or_create("user1");
        pool.get_or_create("user2");
        pool.get_or_create("user3");

        let stats = pool.stats();
        assert_eq!(stats.total_actors, 3);
        assert_eq!(stats.stripe_sizes.len(), NUM_STRIPES);
    }

    #[test]
    fn test_stripe_distribution() {
        let pool = ActorPool::new(test_rules());

        // Add many users
        for i in 0..1000 {
            pool.get_or_create(&format!("user{}", i));
        }

        let stats = pool.stats();
        assert_eq!(stats.total_actors, 1000);

        // Check that distribution is somewhat even
        let avg = 1000.0 / NUM_STRIPES as f64;
        let max_stripe = *stats.stripe_sizes.iter().max().unwrap() as f64;
        let min_stripe = *stats.stripe_sizes.iter().min().unwrap() as f64;

        // Allow 5x deviation from average (reasonable for hash distribution)
        assert!(max_stripe < avg * 5.0, "max stripe too large: {}", max_stripe);
        assert!(min_stripe > avg / 5.0 || min_stripe == 0.0, "min stripe too small: {}", min_stripe);
    }
}
