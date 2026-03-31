# LibreFang Rust SDK

Official Rust client for the LibreFang Agent OS REST API.

## Installation

```toml
[dependencies]
librefang = "2026.4"
tokio = { version = "1", features = ["full"] }
```

## Usage

```rust
use librefang::LibreFang;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = LibreFang::new("http://localhost:4545");

    // List skills
    let skills = client.skills().list().await?;
    println!("Skills: {}", skills.skills.len());

    // List models
    let models = client.models().list().await?;
    println!("Models: {}", models.models.len());

    // List providers
    let providers = client.providers().list().await?;
    println!("Providers: {}", providers.providers.len());

    // Create an agent
    let agent = client.agents()
        .create(librefang::agents::CreateAgentRequest {
            template: Some("assistant".to_string()),
            name: None,
        })
        .await?;
    println!("Created agent: {}", agent.id);

    // Send a message
    let response = client.agents()
        .message(&agent.id, "Hello!")
        .await?;
    println!("Response: {}", response.response);

    // Stream a response
    use futures::stream::StreamExt;
    let response = client.agents().stream(&agent.id, "Tell me a joke").await?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if let Ok(bytes) = chunk {
            print!("{}", String::from_utf8_lossy(&bytes));
        }
    }
    println!();

    Ok(())
}
```

## API

### Client

- `LibreFang::new(base_url)` - Create a new client

### Resources

- `client.agents()` - Agent operations
  - `list()` - List all agents
  - `get(id)` - Get agent by ID
  - `create(request)` - Create an agent
  - `delete(id)` - Delete an agent
  - `message(id, text)` - Send a message
  - `stream(id, text)` - Stream a response

- `client.skills()` - Skill management
  - `list()` - List all skills
  - `install(name)` - Install a skill
  - `uninstall(name)` - Uninstall a skill

- `client.models()` - Model management
  - `list()` - List all models

- `client.providers()` - Provider configuration
  - `list()` - List all providers

## License

MIT
