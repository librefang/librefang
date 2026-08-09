use librefang::LibreFang;
use std::env::VarError;

fn resolve_base_url(configured: Result<String, VarError>) -> Result<String, VarError> {
    match configured {
        Ok(base_url) => Ok(base_url),
        Err(VarError::NotPresent) => Ok("http://127.0.0.1:4545".to_string()),
        Err(error @ VarError::NotUnicode(_)) => Err(error),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base_url = resolve_base_url(std::env::var("LIBREFANG_URL"))?;
    let client = LibreFang::new(base_url);

    // List skills
    let skills = client.skills.list_skills().await?;
    println!(
        "Skills: {}",
        skills["skills"].as_array().map(|a| a.len()).unwrap_or(0)
    );

    // List models
    let models = client.models.list_all_models().await?;
    println!(
        "Models: {}",
        models["models"].as_array().map(|a| a.len()).unwrap_or(0)
    );

    // List providers
    let providers = client.models.list_providers().await?;
    println!(
        "Providers: {}",
        providers["providers"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0)
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve_base_url;

    #[test]
    fn endpoint_uses_configuration_or_local_default() {
        assert_eq!(
            resolve_base_url(Err(std::env::VarError::NotPresent)).unwrap(),
            "http://127.0.0.1:4545".to_string()
        );
        assert_eq!(
            resolve_base_url(Ok("https://agents.example.test".to_string())).unwrap(),
            "https://agents.example.test".to_string()
        );
        assert!(matches!(
            resolve_base_url(Err(std::env::VarError::NotUnicode(
                "invalid endpoint encoding".into()
            ))),
            Err(std::env::VarError::NotUnicode(_))
        ));
    }
}
