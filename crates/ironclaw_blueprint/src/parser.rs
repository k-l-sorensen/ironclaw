//! Parse + validate blueprint source into a [`Blueprint`] AST.
//!
//! The pipeline is ordered to fail loud on the highest-signal problem first:
//!
//! 1. parse to an untyped [`toml::Value`] (syntax errors);
//! 2. scan every string for inline secrets (catches credentials even in keys
//!    that step 3 would reject as unknown);
//! 3. deserialize into the typed AST (`deny_unknown_fields` → unknown keys);
//! 4. semantic validation (api_version major, identifiers, harness shape).

use crate::error::BlueprintError;
use crate::schema::{Blueprint, BlueprintKind};
use crate::secret_scan;

/// The api_version this build understands. A `parse` of any other major is a
/// hard error — schema majors are forever and require a migration path.
pub const SUPPORTED_API_VERSION: &str = "ironclaw.config/v1";

/// Parse and fully validate blueprint source text.
pub fn parse(source: &str) -> Result<Blueprint, BlueprintError> {
    let value: toml::Value = toml::from_str(source)?;
    secret_scan::scan(&value)?;
    let blueprint: Blueprint = value.try_into()?;
    validate(&blueprint)?;
    Ok(blueprint)
}

fn validate(blueprint: &Blueprint) -> Result<(), BlueprintError> {
    validate_api_version(&blueprint.api_version)?;
    // `kind` is enforced by the `BlueprintKind` enum during deserialization;
    // the explicit match keeps the intent legible and survives new variants.
    match blueprint.kind {
        BlueprintKind::Blueprint => {}
    }

    validate_scope(&blueprint.scope)?;

    for (index, extension) in blueprint.extensions.iter().enumerate() {
        validate_identifier(&format!("extensions[{index}].id"), &extension.id)?;
    }
    for (index, skill) in blueprint.skills.iter().enumerate() {
        validate_identifier(&format!("skills[{index}].id"), &skill.id)?;
    }
    for (index, mission) in blueprint.missions.iter().enumerate() {
        validate_identifier(&format!("missions[{index}].id"), &mission.id)?;
    }
    for (index, project) in blueprint.projects.iter().enumerate() {
        validate_identifier(&format!("projects[{index}].id"), &project.id)?;
    }

    if let Some(harness) = &blueprint.harness {
        if harness.id.is_some() && harness.inline.is_some() {
            return Err(BlueprintError::InvalidIdentifier {
                path: "harness".to_string(),
                value: "id + inline".to_string(),
                reason: "bind a registered harness by `id` or define one `inline`, not both"
                    .to_string(),
            });
        }
        if let Some(id) = &harness.id {
            validate_identifier("harness.id", id)?;
        }
        if let Some(inline) = &harness.inline {
            validate_identifier("harness.inline.id", &inline.id)?;
            for (index, required) in inline.required_extensions.iter().enumerate() {
                validate_identifier(
                    &format!("harness.inline.required_extensions[{index}].id"),
                    &required.id,
                )?;
            }
            for (index, required) in inline.required_skills.iter().enumerate() {
                validate_identifier(
                    &format!("harness.inline.required_skills[{index}].id"),
                    &required.id,
                )?;
            }
        }
    }

    Ok(())
}

fn validate_api_version(found: &str) -> Result<(), BlueprintError> {
    if found == SUPPORTED_API_VERSION {
        return Ok(());
    }
    // Accept any patch/minor within the supported major but reject a
    // different major. The version string is `ironclaw.config/v<major>`.
    let supported_major = SUPPORTED_API_VERSION.rsplit("/v").next();
    let found_major = found.rsplit("/v").next();
    match (supported_major, found_major) {
        (Some(want), Some(got))
            if found.starts_with("ironclaw.config/v") && major_matches(want, got) =>
        {
            Ok(())
        }
        _ => Err(BlueprintError::UnsupportedApiVersion {
            found: found.to_string(),
        }),
    }
}

fn major_matches(want: &str, got: &str) -> bool {
    let major_of = |segment: &str| segment.split('.').next().unwrap_or(segment).to_string();
    major_of(want) == major_of(got)
}

fn validate_scope(scope: &crate::schema::Scope) -> Result<(), BlueprintError> {
    for (field, value) in [
        ("scope.tenant", &scope.tenant),
        ("scope.user", &scope.user),
        ("scope.project", &scope.project),
        ("scope.agent", &scope.agent),
    ] {
        if let Some(value) = value {
            validate_identifier(field, value)?;
        }
    }
    Ok(())
}

/// Identifiers must be non-empty, bounded, and free of path separators and
/// whitespace — mirrors the scope/name validators in `ironclaw_host_api`.
fn validate_identifier(path: &str, value: &str) -> Result<(), BlueprintError> {
    let invalid = |reason: &str| BlueprintError::InvalidIdentifier {
        path: path.to_string(),
        value: value.to_string(),
        reason: reason.to_string(),
    };
    if value.is_empty() {
        return Err(invalid("empty identifier"));
    }
    if value.len() > 128 {
        return Err(invalid("longer than 128 bytes"));
    }
    if value.contains("..") {
        return Err(invalid("contains `..`"));
    }
    for character in value.chars() {
        let ok = character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.');
        if !ok {
            return Err(invalid(
                "contains a character outside `a-zA-Z0-9_-.` (no spaces, slashes, or control chars)",
            ));
        }
    }
    Ok(())
}
