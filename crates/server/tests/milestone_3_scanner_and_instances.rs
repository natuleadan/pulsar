use agents_protocol::ProviderProtocol;
use agents_server::modules::{
    plugins::scanner::{
        DiagnosticStatus, ExtensionDoctor, InstalledFileRecord, PluginInstallManifest,
    },
    providers::instances::{AuthCredential, ProviderInstance, ProviderInstanceManager},
};

#[test]
fn extension_doctor_audits_manifest_and_cleans_safely() {
    let tmp_path = std::env::temp_dir().join(format!("agents_doc_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_path);
    std::fs::create_dir_all(&tmp_path).unwrap();
    let base = &tmp_path;

    // Create mock files
    let f1 = base.join("plugin.json");
    let f2 = base.join("tool.wasm");
    std::fs::write(&f1, "{}").unwrap();
    std::fs::write(&f2, "binary").unwrap();

    let manifest = PluginInstallManifest {
        plugin_id: "test-plugin".into(),
        version: "1.0.0".into(),
        files: vec![
            InstalledFileRecord {
                relative_path: "plugin.json".into(),
                sha256_hash: "hash1".into(),
                size_bytes: 2,
            },
            InstalledFileRecord {
                relative_path: "tool.wasm".into(),
                sha256_hash: "hash2".into(),
                size_bytes: 6,
            },
            InstalledFileRecord {
                relative_path: "missing.txt".into(),
                sha256_hash: "hash3".into(),
                size_bytes: 0,
            },
        ],
        installed_at_ms: 1000,
    };

    let diagnostics = ExtensionDoctor::audit_plugin_manifest(&manifest, base);
    assert_eq!(diagnostics.len(), 3);
    assert_eq!(diagnostics[0].status, DiagnosticStatus::Pass);
    assert_eq!(diagnostics[1].status, DiagnosticStatus::Pass);
    assert_eq!(diagnostics[2].status, DiagnosticStatus::Fail);

    // Uninstall
    let removed = ExtensionDoctor::uninstall_plugin_safely(&manifest, base).unwrap();
    assert_eq!(removed, 2);
    assert!(!f1.exists());
    assert!(!f2.exists());
    let _ = std::fs::remove_dir_all(&tmp_path);
}

#[tokio::test]
async fn provider_instances_segregate_by_workgroup() {
    let manager = ProviderInstanceManager::new();

    // Register personal instance
    manager
        .register_instance(ProviderInstance {
            id: "inst-openai-personal".into(),
            name: "Personal OpenAI".into(),
            protocol: ProviderProtocol::OpenAiCompatible,
            base_url: "https://api.openai.com".into(),
            credential: AuthCredential::ApiKey("sk-personal".into()),
            workgroup_id: None,
            max_monthly_budget_usd: Some(50.0),
            is_active: true,
        })
        .await
        .unwrap();

    // Register corporate instance for workgroup-acme
    manager
        .register_instance(ProviderInstance {
            id: "inst-openai-corp".into(),
            name: "Acme Corp OpenAI".into(),
            protocol: ProviderProtocol::OpenAiCompatible,
            base_url: "https://api.openai.com".into(),
            credential: AuthCredential::OAuthToken {
                access_token: "oauth_access_token".into(),
                refresh_token: "oauth_refresh_token".into(),
                expires_at_ms: 2000000000,
            },
            workgroup_id: Some("workgroup-acme".into()),
            max_monthly_budget_usd: Some(5000.0),
            is_active: true,
        })
        .await
        .unwrap();

    // Resolve for workgroup-acme
    let corp = manager
        .resolve_for_workgroup("workgroup-acme", &ProviderProtocol::OpenAiCompatible)
        .await
        .unwrap();
    assert_eq!(corp.id, "inst-openai-corp");

    // Resolve for unassigned workgroup -> falls back to global personal instance
    let fallback = manager
        .resolve_for_workgroup("workgroup-other", &ProviderProtocol::OpenAiCompatible)
        .await
        .unwrap();
    assert_eq!(fallback.id, "inst-openai-personal");
}
