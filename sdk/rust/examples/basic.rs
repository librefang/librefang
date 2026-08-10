use librefang::LibreFang;
use serde_json::Value;
use std::io::{Error, ErrorKind};

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = LibreFang::new("http://127.0.0.1:4545");

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
    use super::expected_array_len;
    use serde_json::json;

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
