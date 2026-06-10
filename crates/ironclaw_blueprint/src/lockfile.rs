//! File-reference resolution and the blueprint lockfile.
//!
//! `text_ref` / `brief_ref` values point at files alongside the blueprint
//! (prompt bodies, mission briefs). The epic requires them to be resolved
//! relative to the blueprint root, read once, and embedded in a lockfile by
//! SHA-256 so an apply is reproducible and tamper-evident across machines.
//!
//! Resolution fails closed: absolute paths and any `..` component are rejected
//! before touching the filesystem, so a blueprint cannot reach outside its own
//! directory.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Blueprint;
use crate::error::BlueprintError;

/// One file referenced by the blueprint, with the AST path that referenced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRefSite {
    /// Dotted AST path of the field holding the reference (e.g.
    /// `system_prompt.text_ref`). Used for error messages.
    pub site: String,
    /// The root-relative reference string as written in the blueprint.
    pub reference: String,
}

/// A resolved, hashed file reference recorded in the lockfile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedFile {
    /// Root-relative path of the referenced file.
    pub path: String,
    /// Lowercase hex SHA-256 of the file contents.
    pub sha256: String,
}

/// The blueprint lockfile: the api_version it was produced from plus every
/// referenced file with its content hash, sorted by path for determinism.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lockfile {
    pub api_version: String,
    pub files: Vec<LockedFile>,
}

impl Blueprint {
    /// Collect every file reference in the document, in declaration order.
    pub fn file_refs(&self) -> Vec<FileRefSite> {
        let mut refs = Vec::new();
        if let Some(prompt) = &self.system_prompt {
            refs.push(FileRefSite {
                site: "system_prompt.text_ref".to_string(),
                reference: prompt.text_ref.clone(),
            });
        }
        for (index, mission) in self.missions.iter().enumerate() {
            if let Some(brief) = &mission.brief_ref {
                refs.push(FileRefSite {
                    site: format!("missions[{index}].brief_ref"),
                    reference: brief.clone(),
                });
            }
        }
        if let Some(harness) = &self.harness
            && let Some(inline) = &harness.inline
            && let Some(overlay) = &inline.prompt_overlay
        {
            refs.push(FileRefSite {
                site: "harness.inline.prompt_overlay.text_ref".to_string(),
                reference: overlay.text_ref.clone(),
            });
        }
        refs
    }

    /// Resolve every file reference against `root`, hash the contents, and
    /// produce a [`Lockfile`]. Fails if a reference escapes the root or a
    /// referenced file cannot be read.
    pub fn resolve_lockfile(&self, root: &Path) -> Result<Lockfile, BlueprintError> {
        let mut files = Vec::new();
        for FileRefSite { site, reference } in self.file_refs() {
            let relative = validate_relative(&site, &reference)?;
            let absolute = root.join(&relative);
            let bytes = std::fs::read(&absolute).map_err(|e| BlueprintError::FileRefRead {
                path: site.clone(),
                reference: reference.clone(),
                reason: e.to_string(),
            })?;
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            files.push(LockedFile {
                path: reference,
                sha256: hex::encode(hasher.finalize()),
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files.dedup();
        Ok(Lockfile {
            api_version: self.api_version.clone(),
            files,
        })
    }
}

/// Reject absolute paths, root/prefix components, and any `..` so a reference
/// can only name files at or below the blueprint root.
fn validate_relative(site: &str, reference: &str) -> Result<PathBuf, BlueprintError> {
    let invalid = |reason: &str| BlueprintError::InvalidFileRef {
        path: site.to_string(),
        reference: reference.to_string(),
        reason: reason.to_string(),
    };

    let path = Path::new(reference);
    if path.is_absolute() {
        return Err(invalid("absolute paths are not allowed"));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => normalized.push(segment),
            Component::CurDir => {}
            Component::ParentDir => return Err(invalid("`..` components are not allowed")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(invalid("absolute paths are not allowed"));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(invalid("empty reference"));
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_absolute_ref() {
        let err = validate_relative("system_prompt.text_ref", "/etc/passwd")
            .expect_err("absolute rejected");
        assert!(matches!(err, BlueprintError::InvalidFileRef { .. }));
    }

    #[test]
    fn rejects_parent_traversal() {
        let err = validate_relative("system_prompt.text_ref", "../../secrets.txt")
            .expect_err("traversal rejected");
        assert!(matches!(err, BlueprintError::InvalidFileRef { .. }));
    }

    #[test]
    fn accepts_nested_relative() {
        let resolved = validate_relative("system_prompt.text_ref", "files/prompt.md")
            .expect("nested relative ok");
        assert_eq!(resolved, PathBuf::from("files/prompt.md"));
    }
}
