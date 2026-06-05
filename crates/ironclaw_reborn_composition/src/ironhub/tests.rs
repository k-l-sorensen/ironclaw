use ironclaw_host_api::sha256_digest_token;
use ironclaw_product_workflow::{LifecyclePhase, LifecycleProductPayload, LifecycleSkillSource};

use crate::lifecycle::response_with_payload;

use super::catalog::{
    classify_gate_and_digest, host_is_disallowed_target, skill_summary, tool_artifact_digest,
    tool_summary, verify_signed_manifest_with_keys,
};
use super::model::{
    IronHubArtifact, IronHubEntryKind, IronHubInstallOptions, IronHubManifest, IronHubProvenance,
    IronHubSkillEntry, IronHubToolEntry,
};
use super::render::render_reborn_ironhub_response;

#[test]
fn signed_manifest_verifies_known_test_vector() {
    let envelope = br#"{"v":1,"key_id":"test-vector","manifest_b64":"eyJ2ZXJzaW9uIjoiMSIsImdlbmVyYXRlZF9hdCI6IjIwMjYtMDEtMDFUMDA6MDA6MDBaIiwicmVsZWFzZV90YWciOiJ0ZXN0IiwicmVwbyI6Im5lYXJhaS9pcm9uaHViIiwidG9vbHMiOltdLCJza2lsbHMiOltdfQ","sig":"KjsUDgi1enj3iTPNQI6gU1Bwxf01hIUItlFvX9PxgWNybPPrJNIV7vFG-G8hJOalFMwFs5zQHrxbtFDZAlgtBg"}"#;
    let manifest = verify_signed_manifest_with_keys(
        envelope,
        &[(
            "test-vector",
            "ca46572f4dcd485599cdf95442934a3e3c86e2cae766a85fbffc8d6540959928",
        )],
    )
    .expect("signed manifest verifies");

    assert_eq!(
            manifest,
            br#"{"version":"1","generated_at":"2026-01-01T00:00:00Z","release_tag":"test","repo":"nearai/ironhub","tools":[],"skills":[]}"#
        );
}

#[test]
fn missing_provenance_defaults_to_unverified() {
    let manifest: IronHubManifest = serde_json::from_str(
        r#"{
                "version": "1",
                "generated_at": "2026-01-01T00:00:00Z",
                "release_tag": "test",
                "repo": "nearai/ironhub",
                "tools": [{
                    "name": "community-tool",
                    "crate_name": "community-tool",
                    "version": "0.1.0",
                    "description": "community",
                    "wasm": {
                        "url": "https://hub.ironclaw.com/community-tool.wasm",
                        "size_bytes": 1,
                        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    },
                    "capabilities": {
                        "url": "https://hub.ironclaw.com/community-tool.capabilities.json",
                        "size_bytes": 1,
                        "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    }
                }],
                "skills": [{
                    "name": "community-skill",
                    "version": "0.1.0",
                    "description": "community",
                    "skill_md": {
                        "url": "https://hub.ironclaw.com/community-skill/SKILL.md",
                        "size_bytes": 1,
                        "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                    }
                }]
            }"#,
    )
    .expect("manifest parses");

    assert_eq!(manifest.tools[0].provenance, IronHubProvenance::New);
    assert_eq!(manifest.skills[0].provenance, IronHubProvenance::New);
}

#[test]
fn unverified_install_requires_acknowledgement() {
    let manifest = IronHubManifest {
        version: "1".to_string(),
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        release_tag: "test".to_string(),
        repo: "nearai/ironhub".to_string(),
        tools: Vec::new(),
        skills: vec![IronHubSkillEntry {
            name: "community-skill".to_string(),
            trunk: String::new(),
            version: "0.1.0".to_string(),
            description: String::new(),
            provenance: IronHubProvenance::New,
            skill_md: IronHubArtifact {
                url: "https://hub.ironclaw.com/community-skill/SKILL.md".to_string(),
                size_bytes: 1,
                sha256: "c".repeat(64),
            },
        }],
    };

    let blocked = classify_gate_and_digest(
        &manifest,
        "community-skill",
        Some(IronHubEntryKind::Skill),
        &IronHubInstallOptions::default(),
    )
    .expect_err("unverified content requires acknowledgement");
    assert!(blocked.to_string().contains("UNVERIFIED community content"));

    let allowed = classify_gate_and_digest(
        &manifest,
        "community-skill",
        Some(IronHubEntryKind::Skill),
        &IronHubInstallOptions {
            acknowledge_unverified: true,
            ..IronHubInstallOptions::default()
        },
    )
    .expect("acknowledged unverified content can proceed");
    assert_eq!(allowed.0, IronHubEntryKind::Skill);
    assert_eq!(allowed.1, IronHubProvenance::New);
}

#[test]
fn renderer_includes_tools_and_skills_in_mixed_search() {
    let skill = skill_summary(&IronHubSkillEntry {
        name: "reviewer".to_string(),
        trunk: String::new(),
        version: "0.2.0".to_string(),
        description: "review skill".to_string(),
        provenance: IronHubProvenance::Verified,
        skill_md: IronHubArtifact {
            url: "https://hub.ironclaw.com/reviewer/SKILL.md".to_string(),
            size_bytes: 1,
            sha256: "c".repeat(64),
        },
    })
    .expect("skill summary");
    assert_eq!(skill.source, LifecycleSkillSource::Registry);

    let response = response_with_payload(
        None,
        LifecyclePhase::Discovered,
        LifecycleProductPayload::CatalogSearch {
            count: 2,
            tools: vec![
                tool_summary(&IronHubToolEntry {
                    name: "web".to_string(),
                    crate_name: "web-tool".to_string(),
                    version: "0.1.0".to_string(),
                    description: "web tool".to_string(),
                    provenance: IronHubProvenance::Official,
                    wasm: IronHubArtifact {
                        url: "https://hub.ironclaw.com/web.wasm".to_string(),
                        size_bytes: 1,
                        sha256: "a".repeat(64),
                    },
                    capabilities: IronHubArtifact {
                        url: "https://hub.ironclaw.com/web.capabilities.json".to_string(),
                        size_bytes: 1,
                        sha256: "b".repeat(64),
                    },
                })
                .expect("tool summary"),
            ],
            skills: vec![skill],
        },
    );

    let rendered = render_reborn_ironhub_response("search", &response);
    assert!(rendered.contains("- tool web 0.1.0"));
    assert!(rendered.contains("- skill reviewer 0.2.0"));
}

#[test]
fn artifact_digest_binds_both_tool_artifacts() {
    let tool = IronHubToolEntry {
        name: "web".to_string(),
        crate_name: "web-tool".to_string(),
        version: "0.1.0".to_string(),
        description: String::new(),
        provenance: IronHubProvenance::Official,
        wasm: IronHubArtifact {
            url: "https://hub.ironclaw.com/web.wasm".to_string(),
            size_bytes: 1,
            sha256: "a".repeat(64),
        },
        capabilities: IronHubArtifact {
            url: "https://hub.ironclaw.com/web.capabilities.json".to_string(),
            size_bytes: 1,
            sha256: "b".repeat(64),
        },
    };
    assert_eq!(
        tool_artifact_digest(&tool),
        sha256_digest_token(format!("{}:{}", "a".repeat(64), "b".repeat(64)).as_bytes())
    );
}

#[test]
fn artifact_url_rejects_internal_hosts_even_when_extra() {
    assert!(host_is_disallowed_target("localhost"));
    assert!(host_is_disallowed_target("10.0.0.1"));
    assert!(host_is_disallowed_target("service.internal"));
}
