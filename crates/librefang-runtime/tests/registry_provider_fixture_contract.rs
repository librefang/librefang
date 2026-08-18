use librefang_types::model_catalog::ModelCatalogFile;

#[test]
fn bedrock_fixture_uses_the_driver_bearer_token_environment_variable() {
    let source = include_str!("fixtures/registry/providers/bedrock.toml");
    let catalog = toml::from_str::<ModelCatalogFile>(source)
        .expect("Bedrock provider fixture must parse as a model catalog");
    let provider = catalog
        .provider
        .expect("Bedrock provider fixture must declare provider metadata");

    assert_eq!(provider.id, "bedrock");
    assert_eq!(provider.api_key_env, "AWS_BEARER_TOKEN_BEDROCK");
}
