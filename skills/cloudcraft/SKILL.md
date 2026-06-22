# Cloudcraft Skill

Visualize and manage cloud architecture diagrams using Cloudcraft.

## Capabilities

- **Inventory**: Scan cloud environments (AWS/Azure) for resources.
- **Diagramming**: Create and manage high-quality architecture diagrams.
- **Snapshots**: Capture live infrastructure state as images (PNG/SVG).
- **Automation**: Programmatic diagram generation and updates.

## Usage

The `cloudcraft` command is available within this skill.

### Commands

```bash
cloudcraft me                      # Show current user profile and settings
cloudcraft blueprints              # List all available architecture blueprints
cloudcraft aws-list                # List linked AWS accounts
cloudcraft aws-snapshot <id> <reg> # Create a snapshot of an AWS account
```

### Examples

**List diagrams**:
```bash
cloudcraft blueprints
```

**Get user info**:
```bash
cloudcraft me
```

## Configuration

Requires `CLOUDCRAFT_API_KEY` environment variable.
Current account: Mission Control (mission@activestyle.sk)
