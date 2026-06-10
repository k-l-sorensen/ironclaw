//! End-to-end parser tests against a full `ironclaw.config/v1` document,
//! exercising the acceptance criteria for epic #3036 slice 1.

use ironclaw_blueprint::{BlueprintError, parse, to_toml};

const FULL: &str = r#"
api_version = "ironclaw.config/v1"
kind = "Blueprint"

[scope]
tenant = "acme"

[system_prompt]
text_ref = "files/system_prompt.md"
applies_to = { project = "*" }

[providers]
default_llm = "anthropic"

[providers.anthropic]
model = "claude-opus-4-7"
api_key = "${secret:anthropic_api_key}"

[runtime]
profile = "HostedDev"
approval_policy = "ask_destructive"

[agent_loop]
run_class = "interactive_coding"
loop_driver = "lightweight"

[[extensions]]
id = "github-mcp"
version = "^0.4"
trust = "user_trusted"
config = { default_org = "acme-corp" }
auth = { kind = "oauth", account_ref = "github.work" }

[[skills]]
id = "code-review"
source = "registry"
enabled = true

[[missions]]
id = "weekly-security-sweep"
schedule = "0 9 * * MON"
brief_ref = "files/missions/security-sweep.md"

[[projects]]
id = "acme-monorepo"
seed = { from = "git", url = "https://github.com/acme/monorepo", ref = "main" }

[capability_surface]
allow = ["filesystem.read", "github-mcp.*", "memory.*"]
deny = ["shell.*"]

[harness]
id = "chain-incident-response"
"#;

#[test]
fn parses_full_document() {
    let blueprint = parse(FULL).expect("full blueprint parses");
    assert_eq!(blueprint.scope.tenant.as_deref(), Some("acme"));
    assert_eq!(blueprint.extensions.len(), 1);
    assert_eq!(blueprint.extensions[0].id, "github-mcp");
    let providers = blueprint.providers.as_ref().expect("providers present");
    assert_eq!(providers.default_llm.as_deref(), Some("anthropic"));
    assert!(providers.entries.contains_key("anthropic"));
}

#[test]
fn round_trip_is_stable() {
    let first = parse(FULL).expect("parses");
    let reemitted = to_toml(&first).expect("serializes");
    let second = parse(&reemitted).expect("re-parses");
    assert_eq!(first, second, "parse -> emit -> parse must be identical");
}

#[test]
fn rejects_unknown_top_level_key() {
    let src = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\nbogus = true\n";
    let err = parse(src).expect_err("unknown key rejected");
    assert!(matches!(err, BlueprintError::Toml(_)));
}

#[test]
fn rejects_unknown_nested_key() {
    let src = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
               [runtime]\nprofile = \"HostedDev\"\nmystery = 1\n";
    let err = parse(src).expect_err("unknown nested key rejected");
    assert!(matches!(err, BlueprintError::Toml(_)));
}

#[test]
fn rejects_wrong_api_version_major() {
    let src = "api_version = \"ironclaw.config/v2\"\nkind = \"Blueprint\"\n";
    let err = parse(src).expect_err("wrong major rejected");
    assert!(matches!(err, BlueprintError::UnsupportedApiVersion { .. }));
}

#[test]
fn rejects_inline_secret_pointing_at_path() {
    let src = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
               [providers.anthropic]\napi_key = \"sk-proj-abcdef1234567890abcdef1234\"\n";
    let err = parse(src).expect_err("inline secret rejected");
    match err {
        BlueprintError::InlineSecret { path, .. } => {
            assert_eq!(path, "providers.anthropic.api_key");
        }
        other => panic!("expected InlineSecret, got {other:?}"),
    }
}

#[test]
fn rejects_both_harness_id_and_inline() {
    let src = "api_version = \"ironclaw.config/v1\"\nkind = \"Blueprint\"\n\
               [harness]\nid = \"x\"\n[harness.inline]\nid = \"y\"\n";
    let err = parse(src).expect_err("ambiguous harness rejected");
    assert!(matches!(err, BlueprintError::InvalidIdentifier { .. }));
}

#[test]
fn resolves_lockfile_with_sha256() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("files/missions")).expect("mkdir");
    std::fs::write(
        dir.path().join("files/system_prompt.md"),
        b"You are Acme.\n",
    )
    .expect("write");
    std::fs::write(
        dir.path().join("files/missions/security-sweep.md"),
        b"Sweep weekly.\n",
    )
    .expect("write");

    let blueprint = parse(FULL).expect("parses");
    let lock = blueprint
        .resolve_lockfile(dir.path())
        .expect("lockfile resolves");
    assert_eq!(lock.api_version, "ironclaw.config/v1");
    assert_eq!(lock.files.len(), 2);
    // Sorted by path; every hash is 64 lowercase hex chars.
    assert_eq!(lock.files[0].path, "files/missions/security-sweep.md");
    for file in &lock.files {
        assert_eq!(file.sha256.len(), 64);
        assert!(file.sha256.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

#[test]
fn lockfile_rejects_missing_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let blueprint = parse(FULL).expect("parses");
    let err = blueprint
        .resolve_lockfile(dir.path())
        .expect_err("missing file rejected");
    assert!(matches!(err, BlueprintError::FileRefRead { .. }));
}
