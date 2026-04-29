use librefang::LibreFang;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = LibreFang::new("http://127.0.0.1:4545");

    // List skills
    let skills = client.skills.list_skills().await?;
    println!("Skills: {}", skills["skills"].as_array().map(|a| a.len()).unwrap_or(0));

    // List models
    let models = client.models.list_all_models().await?;
    println!("Models: {}", models["models"].as_array().map(|a| a.len()).unwrap_or(0));

    // List providers
    let providers = client.models.list_providers().await?;
    println!("Providers: {}", providers["providers"].as_array().map(|a| a.len()).unwrap_or(0));

    Ok(())
}
