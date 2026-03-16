use librefang::LibreFang;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = LibreFang::new("http://127.0.0.1:4545");

    // List skills
    let skills = client.skills().list().await?;
    println!("Skills: {}", skills.skills.len());

    // List models
    let models = client.models().list().await?;
    println!("Models: {}", models.models.len());

    // List providers - debug
    let providers = client.providers().list().await?;
    println!("Providers: {}", providers.providers.len());

    Ok(())
}
