//! Explicit, reviewed compatibility policy; never weakens suite assertions.
use crate::{inputs, semantic_diff::Options};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Impact {
    Changed,
    Allowed,
    Breaking,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema_version: u32,
    pub name: String,
    #[serde(default)]
    pub comparison: Options,
    #[serde(default)]
    pub rules: Vec<Rule>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub case: Option<String>,
    pub field: String,
    pub path: Option<String>,
    pub impact: Impact,
    pub reason: String,
}
fn id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}
impl Policy {
    pub fn read(path: &Path) -> Result<Self, String> {
        let bytes = inputs::read_bounded(path, 1024 * 1024)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| "Policy must be UTF-8")?;
        let policy: Self =
            toml::from_str(text).map_err(|e| format!("Invalid policy: {}", e.message()))?;
        policy.validate()?;
        Ok(policy)
    }
    pub fn validate(&self) -> Result<(), String> {
        self.comparison.validate()?;
        let mut ids = BTreeSet::new();
        let mut selectors = BTreeSet::new();
        if self.schema_version != 1 || !id(&self.name) || self.rules.len() > 128 {
            return Err(
                "Policy requires schema_version=1, a valid name and at most 128 rules".into(),
            );
        }
        for rule in &self.rules {
            if !id(&rule.id)
                || !ids.insert(&rule.id)
                || rule.case.as_ref().is_some_and(|v| !id(v))
                || !matches!(
                    rule.field.as_str(),
                    "exit_code"
                        | "stdout"
                        | "stderr"
                        | "workspace"
                        | "contract_status"
                        | "assertion"
                )
                || rule.path.as_ref().is_some_and(|v| v.len() > 4096)
                || rule.reason.trim().is_empty()
                || rule.reason.len() > 256
                || !selectors.insert((&rule.case, &rule.field, &rule.path))
                || (rule.impact == Impact::Allowed
                    && matches!(rule.field.as_str(), "contract_status" | "assertion"))
            {
                return Err(
                    "Invalid/duplicate policy rule; contract/assertion failures cannot be allowed"
                        .into(),
                );
            }
        }
        Ok(())
    }
    pub fn hash(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("serializable policy"))
        )
    }
    pub fn classify(&self, case: &str, field: &str, path: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| {
            r.case.as_deref().is_none_or(|v| v == case)
                && r.field == field
                && r.path.as_deref().is_none_or(|v| v == path)
        })
    }
}
