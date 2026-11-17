use pulsar_protocol::{PluginCapabilityMask, PluginManifest, PluginToolDefinition};
use pulsar_server::modules::{
    mcp::{
        transpiler::OpenJsonTranspiler,
        validator::{ParameterValidator, ValidationError},
    },
    plugins::PluginRegistry,
};
use serde_json::json;

#[test]
fn openjson_transpiler_synthesizes_mcp_tools() {
    let spec = r#"{
        "openapi": "3.0.0",
        "info": { "title": "Pet Store API", "version": "1.0.0" },
        "paths": {
            "/pets": {
                "get": {
                    "operationId": "listPets",
                    "summary": "List all pets",
                    "parameters": [
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ]
                },
                "post": {
                    "operationId": "createPet",
                    "summary": "Create a new pet",
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "required": ["name"],
                                    "properties": {
                                        "name": { "type": "string" },
                                        "tag": { "type": "string" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }"#;

    let tools = OpenJsonTranspiler::transpile(spec).expect("transpilation should succeed");
    assert_eq!(tools.len(), 2);

    let list_tool = tools.iter().find(|t| t.name == "listPets").unwrap();
    assert_eq!(list_tool.method, "GET");
    assert_eq!(list_tool.path, "/pets");
    assert!(list_tool.enabled);

    let create_tool = tools.iter().find(|t| t.name == "createPet").unwrap();
    assert_eq!(create_tool.method, "POST");
    assert!(
        create_tool
            .parameters
            .get("required")
            .unwrap()
            .as_array()
            .unwrap()
            .contains(&json!("name"))
    );
}

#[test]
fn parameter_validator_enforces_schema_types() {
    let schema = json!({
        "type": "object",
        "required": ["id", "count"],
        "properties": {
            "id": { "type": "string" },
            "count": { "type": "integer" },
            "active": { "type": "boolean" }
        }
    });

    // Valid arguments
    let valid_args = json!({
        "id": "item-123",
        "count": 5,
        "active": true
    });
    assert!(ParameterValidator::validate(&valid_args, &schema).is_ok());

    // Missing required argument
    let missing_req = json!({
        "id": "item-123"
    });
    let err = ParameterValidator::validate(&missing_req, &schema).unwrap_err();
    assert_eq!(
        err,
        ValidationError::MissingRequiredProperty("count".into())
    );

    // Type mismatch
    let bad_type = json!({
        "id": "item-123",
        "count": "not-a-number"
    });
    let err_type = ParameterValidator::validate(&bad_type, &schema).unwrap_err();
    match err_type {
        ValidationError::TypeMismatch {
            property,
            expected,
            found,
        } => {
            assert_eq!(property, "count");
            assert_eq!(expected, "integer");
            assert_eq!(found, "string");
        }
        _ => panic!("expected type mismatch error"),
    }
}

#[tokio::test]
async fn plugin_registry_lifecycle_and_agent_card() {
    let registry = PluginRegistry::new();

    let manifest = PluginManifest {
        id: "weather-plugin".into(),
        name: "Weather Plugin".into(),
        version: "1.0.0".into(),
        description: "Live weather data for agents".into(),
        author: "Sovereign Dev".into(),
        permissions: PluginCapabilityMask {
            allow_network: true,
            allowed_hosts: vec!["api.weather.com".into()],
            allow_filesystem: false,
            allowed_paths: vec![],
            allow_env_vars: vec![],
            max_execution_seconds: 10,
            max_memory_mb: 128,
        },
        tools: vec![
            PluginToolDefinition {
                name: "get_current_weather".into(),
                description: "Get weather for location".into(),
                parameters: json!({"type": "object", "required": ["city"], "properties": {"city": {"type": "string"}}}),
                enabled: true,
                path: "/weather".into(),
                method: "GET".into(),
            },
            PluginToolDefinition {
                name: "delete_weather_cache".into(),
                description: "Purge weather cache".into(),
                parameters: json!({"type": "object"}),
                enabled: false,
                path: "/cache".into(),
                method: "DELETE".into(),
            },
        ],
        settings_schema: None,
        metadata: Default::default(),
    };

    assert!(manifest.permissions.is_host_allowed("api.weather.com"));
    assert!(!manifest.permissions.is_host_allowed("malicious.com"));

    registry.register(manifest).await.unwrap();

    let active_tools = registry.collect_active_tools().await;
    assert_eq!(active_tools.len(), 1);
    assert_eq!(active_tools[0].name, "get_current_weather");

    // Enable delete_weather_cache
    registry
        .set_tool_status("weather-plugin", "delete_weather_cache", true)
        .await
        .unwrap();
    let updated_tools = registry.collect_active_tools().await;
    assert_eq!(updated_tools.len(), 2);

    // Agent card generation
    let card = registry.generate_agent_card("https://test.nla.run").await;
    assert_eq!(card.plugins, vec!["weather-plugin".to_owned()]);
    assert_eq!(card.endpoints.mcp_endpoint, "https://test.nla.run/mcp");
    assert!(card.capabilities.contains("mcp:tools"));
}
