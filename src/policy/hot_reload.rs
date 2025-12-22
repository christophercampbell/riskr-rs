use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::interval;
use tracing::{error, info, warn};

use crate::rules::RuleSet;

use super::loader::PolicyLoader;

/// Watch for policy changes and broadcast updates.
pub struct PolicyWatcher {
    loader: PolicyLoader,
    check_interval: Duration,
    last_version: Option<String>,
}

impl PolicyWatcher {
    /// Create a new policy watcher.
    pub fn new(loader: PolicyLoader, check_interval: Duration) -> Self {
        PolicyWatcher {
            loader,
            check_interval,
            last_version: None,
        }
    }

    /// Start watching for policy changes.
    ///
    /// Returns a receiver that will receive new RuleSet instances when
    /// the policy changes.
    pub fn start(mut self) -> (watch::Receiver<Arc<RuleSet>>, tokio::task::JoinHandle<()>) {
        // Load initial policy
        let initial_ruleset = match self.loader.load() {
            Ok((policy, ruleset)) => {
                self.last_version = Some(policy.version.clone());
                info!("Loaded initial policy version: {}", policy.version);
                Arc::new(ruleset)
            }
            Err(e) => {
                error!("Failed to load initial policy: {}", e);
                Arc::new(RuleSet::empty())
            }
        };

        let (tx, rx) = watch::channel(initial_ruleset);

        let handle = tokio::spawn(async move {
            let mut interval = interval(self.check_interval);

            loop {
                interval.tick().await;

                match self.check_for_updates(&tx) {
                    Ok(true) => info!("Policy reloaded successfully"),
                    Ok(false) => {} // No changes
                    Err(e) => warn!("Error checking for policy updates: {}", e),
                }
            }
        });

        (rx, handle)
    }

    /// Check for policy updates and broadcast if changed.
    fn check_for_updates(
        &mut self,
        tx: &watch::Sender<Arc<RuleSet>>,
    ) -> Result<bool, super::loader::PolicyError> {
        let policy = self.loader.load_policy()?;

        // Check if version changed
        if self.last_version.as_ref() == Some(&policy.version) {
            return Ok(false);
        }

        // Reload full policy and sanctions
        let (policy, ruleset) = self.loader.load()?;

        info!(
            "Policy version changed: {:?} -> {}",
            self.last_version, policy.version
        );

        self.last_version = Some(policy.version);
        let _ = tx.send(Arc::new(ruleset));

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_files() -> (NamedTempFile, NamedTempFile) {
        let mut policy_file = NamedTempFile::new().unwrap();
        writeln!(
            policy_file,
            r#"
policy_version: "v1"
params:
  daily_volume_limit_usd: 50000
rules:
  - id: R1_OFAC
    type: ofac_addr
    action: REJECT_FATAL
"#
        )
        .unwrap();

        let mut sanctions_file = NamedTempFile::new().unwrap();
        writeln!(sanctions_file, "0xdead").unwrap();

        (policy_file, sanctions_file)
    }

    #[tokio::test]
    async fn test_policy_watcher_initial_load() {
        let (policy_file, sanctions_file) = create_test_files();

        let loader = PolicyLoader::new(
            policy_file.path().to_string_lossy(),
            sanctions_file.path().to_string_lossy(),
        );

        let watcher = PolicyWatcher::new(loader, Duration::from_secs(60));
        let (rx, handle) = watcher.start();

        let ruleset = rx.borrow();
        assert_eq!(ruleset.policy_version, "v1");
        assert_eq!(ruleset.inline.len(), 1);

        handle.abort();
    }

    #[tokio::test]
    async fn test_policy_watcher_detects_changes() {
        let (policy_file, sanctions_file) = create_test_files();
        let policy_path = policy_file.path().to_path_buf();

        let loader = PolicyLoader::new(
            policy_file.path().to_string_lossy(),
            sanctions_file.path().to_string_lossy(),
        );

        let watcher = PolicyWatcher::new(loader, Duration::from_millis(50));
        let (mut rx, handle) = watcher.start();

        // Initial version
        assert_eq!(rx.borrow().policy_version, "v1");

        // Update policy file
        tokio::time::sleep(Duration::from_millis(10)).await;
        std::fs::write(
            &policy_path,
            r#"
policy_version: "v2"
params:
  daily_volume_limit_usd: 100000
rules:
  - id: R1_OFAC
    type: ofac_addr
    action: REJECT_FATAL
  - id: R2_JURISDICTION
    type: jurisdiction_block
    action: REJECT_FATAL
    blocked_countries: ["IR"]
"#,
        )
        .unwrap();

        // Wait for watcher to detect change
        tokio::time::timeout(Duration::from_secs(1), rx.changed())
            .await
            .expect("Timeout waiting for policy change")
            .unwrap();

        assert_eq!(rx.borrow().policy_version, "v2");
        assert_eq!(rx.borrow().inline.len(), 2);

        handle.abort();
    }
}
