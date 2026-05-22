//! Policy-as-code access control — OPA/Rego style permission checking.
//!
//! This module provides:
//!
//! - [`AccessController`] — a simple policy engine that evaluates
//!   allow/deny rules for operations on keys.
//! - [`AccessPolicy`] — a single policy rule with operation, key pattern,
//!   effect, and optional context matchers.

use std::collections::HashMap;

/// The effect of a policy rule.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Allow the operation.
    Allow,
    /// Deny the operation.
    Deny,
}

/// The type of operation being checked.
#[derive(Debug, Clone, PartialEq, Hash, Eq)]
pub enum Operation {
    /// Read a key.
    Read,
    /// Write a key.
    Write,
    /// Delete a key.
    Delete,
    /// Admin operation.
    Admin,
}

impl std::str::FromStr for Operation {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "read" => Ok(Operation::Read),
            "write" => Ok(Operation::Write),
            "delete" => Ok(Operation::Delete),
            "admin" => Ok(Operation::Admin),
            other => Err(format!("unknown operation: {}", other)),
        }
    }
}

/// A single access-control policy rule.
///
/// Rules are evaluated in order; the first matching rule determines the result.
/// If no rule matches, the default effect is `Deny`.
#[derive(Debug, Clone)]
pub struct AccessPolicy {
    /// A human-readable name for this policy.
    pub name: String,
    /// The operation this rule applies to.
    pub operation: Operation,
    /// A glob-like key pattern (e.g. `"secret/*"`, `"*"`).
    /// Supports `*` as a wildcard matching any sequence of characters.
    pub key_pattern: String,
    /// Whether this rule allows or denies.
    pub effect: Effect,
    /// Optional context matchers as key=value pairs (must all match).
    pub context_matchers: HashMap<String, String>,
}

/// Access controller that evaluates policies in order.
///
/// The first matching policy wins. If no policy matches, access is denied
/// by default.
///
/// # Example
///
/// ```ignore
/// let mut ac = AccessController::new();
/// ac.set_policy("allow_read", AccessPolicy {
///     name: "allow_read".into(),
///     operation: Operation::Read,
///     key_pattern: "*".into(),
///     effect: Effect::Allow,
///     context_matchers: HashMap::new(),
/// });
///
/// let allowed = ac.check_permission(&Operation::Read, b"my_key", &HashMap::new());
/// assert!(allowed);
/// ```
pub struct AccessController {
    policies: Vec<AccessPolicy>,
}

impl AccessController {
    /// Create a new empty access controller (all operations denied by default).
    pub fn new() -> Self {
        Self {
            policies: Vec::new(),
        }
    }

    /// Register (or replace) a policy by name.
    ///
    /// If a policy with the same name already exists, it is replaced.
    /// Policies are evaluated in insertion order.
    pub fn set_policy(&mut self, name: &str, policy: AccessPolicy) {
        if let Some(pos) = self.policies.iter().position(|p| p.name == name) {
            self.policies[pos] = policy;
        } else {
            self.policies.push(policy);
        }
    }

    /// Remove a policy by name.
    pub fn remove_policy(&mut self, name: &str) {
        self.policies.retain(|p| p.name != name);
    }

    /// Check whether an operation on a key is permitted.
    ///
    /// The first matching policy determines the result. If no policy matches,
    /// access is denied.
    ///
    /// * `operation` — the type of operation.
    /// * `key` — the key being accessed.
    /// * `context` — additional key-value context (e.g., `{"role": "admin"}`).
    pub fn check_permission(
        &self,
        operation: &Operation,
        key: &[u8],
        context: &HashMap<String, String>,
    ) -> bool {
        for policy in &self.policies {
            if policy.operation != *operation {
                continue;
            }
            if !self.key_matches_pattern(key, &policy.key_pattern) {
                continue;
            }
            if !self.context_matches(&policy.context_matchers, context) {
                continue;
            }
            return policy.effect == Effect::Allow;
        }
        false // default deny
    }

    /// Return the number of registered policies.
    pub fn policy_count(&self) -> usize {
        self.policies.len()
    }

    /// Simple glob matching: `*` matches any sequence of characters.
    fn key_matches_pattern(&self, key: &[u8], pattern: &str) -> bool {
        let key_str = String::from_utf8_lossy(key);
        if pattern == "*" {
            return true;
        }
        if let Some(suffix) = pattern.strip_suffix('*') {
            key_str.starts_with(suffix)
        } else if let Some(prefix) = pattern.strip_prefix('*') {
            key_str.ends_with(prefix)
        } else {
            key_str == pattern
        }
    }

    /// Check that all context matchers are satisfied.
    fn context_matches(
        &self,
        matchers: &HashMap<String, String>,
        context: &HashMap<String, String>,
    ) -> bool {
        for (k, v) in matchers {
            match context.get(k) {
                Some(actual) if actual == v => continue,
                _ => return false,
            }
        }
        true
    }
}

impl Default for AccessController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_deny() {
        let ac = AccessController::new();
        assert!(!ac.check_permission(&Operation::Read, b"any_key", &HashMap::new()));
    }

    #[test]
    fn test_allow_all() {
        let mut ac = AccessController::new();
        ac.set_policy(
            "allow_all_read",
            AccessPolicy {
                name: "allow_all_read".into(),
                operation: Operation::Read,
                key_pattern: "*".into(),
                effect: Effect::Allow,
                context_matchers: HashMap::new(),
            },
        );
        assert!(ac.check_permission(&Operation::Read, b"anything", &HashMap::new()));
        assert!(!ac.check_permission(&Operation::Write, b"anything", &HashMap::new()));
    }

    #[test]
    fn test_key_prefix_pattern() {
        let mut ac = AccessController::new();
        ac.set_policy(
            "secret_read",
            AccessPolicy {
                name: "secret_read".into(),
                operation: Operation::Read,
                key_pattern: "secret/*".into(),
                effect: Effect::Allow,
                context_matchers: HashMap::new(),
            },
        );
        assert!(ac.check_permission(&Operation::Read, b"secret/config", &HashMap::new()));
        assert!(!ac.check_permission(&Operation::Read, b"public/config", &HashMap::new()));
    }

    #[test]
    fn test_context_matchers() {
        let mut ac = AccessController::new();
        let mut matchers = HashMap::new();
        matchers.insert("role".to_string(), "admin".to_string());
        ac.set_policy(
            "admin_write",
            AccessPolicy {
                name: "admin_write".into(),
                operation: Operation::Write,
                key_pattern: "*".into(),
                effect: Effect::Allow,
                context_matchers: matchers,
            },
        );

        let mut admin_ctx = HashMap::new();
        admin_ctx.insert("role".to_string(), "admin".to_string());
        assert!(ac.check_permission(&Operation::Write, b"k", &admin_ctx));

        let user_ctx = HashMap::new();
        assert!(!ac.check_permission(&Operation::Write, b"k", &user_ctx));
    }

    #[test]
    fn test_policy_replacement() {
        let mut ac = AccessController::new();
        ac.set_policy(
            "p1",
            AccessPolicy {
                name: "p1".into(),
                operation: Operation::Read,
                key_pattern: "*".into(),
                effect: Effect::Allow,
                context_matchers: HashMap::new(),
            },
        );
        assert!(ac.check_permission(&Operation::Read, b"x", &HashMap::new()));

        // Replace with deny
        ac.set_policy(
            "p1",
            AccessPolicy {
                name: "p1".into(),
                operation: Operation::Read,
                key_pattern: "*".into(),
                effect: Effect::Deny,
                context_matchers: HashMap::new(),
            },
        );
        assert!(!ac.check_permission(&Operation::Read, b"x", &HashMap::new()));
    }

    #[test]
    fn test_remove_policy() {
        let mut ac = AccessController::new();
        ac.set_policy(
            "temp",
            AccessPolicy {
                name: "temp".into(),
                operation: Operation::Read,
                key_pattern: "*".into(),
                effect: Effect::Allow,
                context_matchers: HashMap::new(),
            },
        );
        assert_eq!(ac.policy_count(), 1);
        ac.remove_policy("temp");
        assert_eq!(ac.policy_count(), 0);
        assert!(!ac.check_permission(&Operation::Read, b"x", &HashMap::new()));
    }
}
