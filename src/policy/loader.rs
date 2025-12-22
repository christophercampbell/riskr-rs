use std::collections::HashSet;
use std::fs;
use std::path::Path;
use thiserror::Error;

use crate::domain::Policy;
use crate::rules::RuleSet;

/// Errors that can occur during policy loading.
#[derive(Error, Debug)]
pub enum PolicyError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML parsing error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("Validation error: {0}")]
    Validation(String),
}

/// Load a policy from a YAML file.
pub fn load_policy(path: impl AsRef<Path>) -> Result<Policy, PolicyError> {
    let content = fs::read_to_string(path)?;
    let policy: Policy = serde_yaml::from_str(&content)?;

    validate_policy(&policy)?;

    Ok(policy)
}

/// Load sanctions list from a text file.
///
/// Expected format: one address per line, # for comments.
pub fn load_sanctions(path: impl AsRef<Path>) -> Result<HashSet<String>, PolicyError> {
    let content = fs::read_to_string(path)?;
    let mut sanctions = HashSet::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Normalize to lowercase
        sanctions.insert(line.to_lowercase());
    }

    Ok(sanctions)
}

/// Validate policy configuration.
fn validate_policy(policy: &Policy) -> Result<(), PolicyError> {
    if policy.version.is_empty() {
        return Err(PolicyError::Validation(
            "Policy version cannot be empty".to_string(),
        ));
    }

    // Check for duplicate rule IDs
    let mut seen_ids = HashSet::new();
    for rule in &policy.rules {
        if !seen_ids.insert(&rule.id) {
            return Err(PolicyError::Validation(format!(
                "Duplicate rule ID: {}",
                rule.id
            )));
        }
    }

    Ok(())
}

/// Policy loader that manages policy and sanctions loading.
pub struct PolicyLoader {
    policy_path: String,
    sanctions_path: String,
}

impl PolicyLoader {
    /// Create a new policy loader.
    pub fn new(policy_path: impl Into<String>, sanctions_path: impl Into<String>) -> Self {
        PolicyLoader {
            policy_path: policy_path.into(),
            sanctions_path: sanctions_path.into(),
        }
    }

    /// Load policy and sanctions, returning a RuleSet.
    pub fn load(&self) -> Result<(Policy, RuleSet), PolicyError> {
        let policy = load_policy(&self.policy_path)?;
        let sanctions = load_sanctions(&self.sanctions_path)?;

        let ruleset = RuleSet::from_policy(&policy, sanctions);

        Ok((policy, ruleset))
    }

    /// Load only the policy (without rebuilding rules).
    pub fn load_policy(&self) -> Result<Policy, PolicyError> {
        load_policy(&self.policy_path)
    }

    /// Load only the sanctions list.
    pub fn load_sanctions(&self) -> Result<HashSet<String>, PolicyError> {
        load_sanctions(&self.sanctions_path)
    }

    /// Get the policy file path.
    pub fn policy_path(&self) -> &str {
        &self.policy_path
    }

    /// Get the sanctions file path.
    pub fn sanctions_path(&self) -> &str {
        &self.sanctions_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_load_policy() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
policy_version: "test-1.0"
params:
  kyc_tier_caps_usd:
    L0: 1000
    L1: 5000
  daily_volume_limit_usd: 50000
rules:
  - id: R1_OFAC
    type: ofac_addr
    action: REJECT_FATAL
  - id: R2_JURISDICTION
    type: jurisdiction_block
    action: REJECT_FATAL
    blocked_countries: ["IR", "KP"]
signature: "unsigned"
"#
        )
        .unwrap();

        let policy = load_policy(file.path()).unwrap();

        assert_eq!(policy.version, "test-1.0");
        assert_eq!(policy.rules.len(), 2);
        assert_eq!(
            policy.params.kyc_tier_caps_usd.get("L0"),
            Some(&rust_decimal::Decimal::new(1000, 0))
        );
    }

    #[test]
    fn test_load_sanctions() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
# OFAC sanctions list
0xDEAD1234567890
0xBEEF0987654321

# Another bad address
0xBAD1111111111
"#
        )
        .unwrap();

        let sanctions = load_sanctions(file.path()).unwrap();

        assert_eq!(sanctions.len(), 3);
        assert!(sanctions.contains("0xdead1234567890")); // Normalized to lowercase
        assert!(sanctions.contains("0xbeef0987654321"));
        assert!(sanctions.contains("0xbad1111111111"));
    }

    #[test]
    fn test_policy_validation_empty_version() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
policy_version: ""
rules: []
"#
        )
        .unwrap();

        let result = load_policy(file.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("version"));
    }

    #[test]
    fn test_policy_validation_duplicate_ids() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
policy_version: "test"
rules:
  - id: R1
    type: ofac_addr
    action: REJECT_FATAL
  - id: R1
    type: jurisdiction_block
    action: REJECT_FATAL
"#
        )
        .unwrap();

        let result = load_policy(file.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Duplicate"));
    }

    #[test]
    fn test_policy_loader() {
        let mut policy_file = NamedTempFile::new().unwrap();
        writeln!(
            policy_file,
            r#"
policy_version: "test-1.0"
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
        writeln!(sanctions_file, "0xdead\n0xbeef").unwrap();

        let loader = PolicyLoader::new(
            policy_file.path().to_string_lossy(),
            sanctions_file.path().to_string_lossy(),
        );

        let (policy, ruleset) = loader.load().unwrap();

        assert_eq!(policy.version, "test-1.0");
        assert_eq!(ruleset.inline.len(), 1);
        assert_eq!(ruleset.policy_version, "test-1.0");
    }
}
