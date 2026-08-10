use librefang::LibreFang;
use serde_json::Value;
use std::env::VarError;
use std::io::{Error, ErrorKind};

fn resolve_base_url(configured: Result<String, VarError>) -> Result<String, VarError> {
    match configured {
        Ok(base_url) => Ok(base_url),
        Err(VarError::NotPresent) => Ok("http://127.0.0.1:4545".to_string()),
        Err(error @ VarError::NotUnicode(_)) => Err(error),
    }
}

fn expected_array_len(response: &Value, key: &str) -> Result<usize, Error> {
    response
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::len)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidData,
                format!("unexpected response shape: expected \"{key}\" array"),
            )
        })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base_url = resolve_base_url(std::env::var("LIBREFANG_URL"))?;
    let client = LibreFang::new(base_url);

    // List skills
    let skills = client.skills.list_skills().await?;
    println!("Skills: {}", expected_array_len(&skills, "items")?);

    // List models
    let models = client.models.list_all_models().await?;
    println!("Models: {}", expected_array_len(&models, "models")?);

    // List providers
    let providers = client.models.list_providers().await?;
    println!(
        "Providers: {}",
        expected_array_len(&providers, "providers")?
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{expected_array_len, resolve_base_url};
    use serde_json::json;

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

    #[test]
    fn response_shape_requires_the_named_array() {
        assert_eq!(
            expected_array_len(
                &json!({
                    "items": [],
                    "total": 0,
                    "offset": 0,
                    "limit": 50,
                    "categories": []
                }),
                "items"
            )
            .unwrap(),
            0
        );
        for response in [json!({}), json!({"items": {}})] {
            let error = expected_array_len(&response, "items").unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
            assert_eq!(
                error.to_string(),
                "unexpected response shape: expected \"items\" array"
            );
        }
    }
}
