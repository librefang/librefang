//! Local skill preparation and packaging helpers for testing and publishing.

use crate::openclaw_compat;
use crate::verify::{SkillVerifier, SkillWarning, WarningSeverity};
use crate::{SkillError, SkillManifest, SkillRuntime};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;

/// The detected format of a local skill source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalSkillFormat {
    /// Native LibreFang skill with a `skill.toml`.
    Native,
    /// OpenClaw `SKILL.md` prompt-only skill.
    SkillMd,
    /// OpenClaw Node.js skill with `package.json`.
    OpenClaw,
}

#[derive(Debug, Clone)]
struct GeneratedSkillFile {
    relative_path: PathBuf,
    contents: Vec<u8>,
}

/// A validated local skill ready for testing or packaging.
#[derive(Debug, Clone)]
pub struct PreparedLocalSkill {
    /// Parsed or converted manifest.
    pub manifest: SkillManifest,
    /// Source directory that contains the skill files.
    pub source_dir: PathBuf,
    /// Original source format.
    pub format: LocalSkillFormat,
    /// Security and prompt-scan warnings gathered during preparation.
    pub warnings: Vec<SkillWarning>,
    generated_files: Vec<GeneratedSkillFile>,
}

impl PreparedLocalSkill {
    /// Return true when any warning is critical.
    pub fn has_critical_warnings(&self) -> bool {
        self.warnings
            .iter()
            .any(|warning| warning.severity == WarningSeverity::Critical)
    }
}

/// A packaged skill bundle ready for upload to FangHub.
#[derive(Debug, Clone)]
pub struct PackagedSkill {
    /// The prepared manifest used to create the archive.
    pub manifest: SkillManifest,
    /// The bundle archive path on disk.
    pub archive_path: PathBuf,
    /// SHA256 of the final archive.
    pub sha256: String,
    /// Final archive size in bytes.
    pub size_bytes: u64,
}

/// Prepare a local skill directory or manifest file for testing or publishing.
pub fn prepare_local_skill(path: &Path) -> Result<PreparedLocalSkill, SkillError> {
    let source_dir = resolve_source_dir(path)?;
    let (manifest, format, generated_files) = load_manifest_from_dir(&source_dir)?;
    validate_manifest(&manifest, &source_dir)?;

    let mut warnings = SkillVerifier::security_scan(&manifest);
    if let Some(ref prompt_context) = manifest.prompt_context {
        warnings.extend(SkillVerifier::scan_prompt_content(prompt_context));
    }

    Ok(PreparedLocalSkill {
        manifest,
        source_dir,
        format,
        warnings,
        generated_files,
    })
}

/// Package a prepared local skill into a zip archive under `output_dir`.
pub fn package_prepared_skill(
    prepared: &PreparedLocalSkill,
    output_dir: &Path,
) -> Result<PackagedSkill, SkillError> {
    std::fs::create_dir_all(output_dir)?;

    let archive_name = format!(
        "{}-{}.zip",
        prepared.manifest.skill.name, prepared.manifest.skill.version
    );
    let archive_path = output_dir.join(archive_name);
    let archive_file = std::fs::File::create(&archive_path)?;
    let mut zip = zip::ZipWriter::new(archive_file);

    let file_options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);

    for entry in WalkDir::new(&prepared.source_dir)
        .into_iter()
        .filter_entry(|entry| should_include_entry(&prepared.source_dir, output_dir, entry.path()))
    {
        let entry = entry.map_err(|err| SkillError::Io(std::io::Error::other(err)))?;
        let path = entry.path();
        if !entry.file_type().is_file() {
            continue;
        }
        if path.starts_with(output_dir) {
            continue;
        }

        let relative_path = path
            .strip_prefix(&prepared.source_dir)
            .map_err(|_| {
                SkillError::InvalidManifest(format!(
                    "Failed to resolve bundle path for {}",
                    path.display()
                ))
            })?
            .to_path_buf();

        let mut file = std::fs::File::open(path)?;
        zip.start_file(normalize_zip_path(&relative_path), file_options)
            .map_err(zip_error)?;
        std::io::copy(&mut file, &mut zip)?;
    }

    for generated in &prepared.generated_files {
        zip.start_file(normalize_zip_path(&generated.relative_path), file_options)
            .map_err(zip_error)?;
        zip.write_all(&generated.contents)?;
    }

    zip.finish().map_err(zip_error)?;

    let archive_bytes = std::fs::read(&archive_path)?;
    Ok(PackagedSkill {
        manifest: prepared.manifest.clone(),
        archive_path,
        sha256: SkillVerifier::sha256_hex(&archive_bytes),
        size_bytes: archive_bytes.len() as u64,
    })
}

fn resolve_source_dir(path: &Path) -> Result<PathBuf, SkillError> {
    if path.is_dir() {
        return Ok(path.to_path_buf());
    }

    if path.is_file() {
        return path.parent().map(Path::to_path_buf).ok_or_else(|| {
            SkillError::InvalidManifest(format!(
                "Could not resolve parent directory for {}",
                path.display()
            ))
        });
    }

    Err(SkillError::NotFound(format!(
        "Skill path not found: {}",
        path.display()
    )))
}

fn load_manifest_from_dir(
    source_dir: &Path,
) -> Result<(SkillManifest, LocalSkillFormat, Vec<GeneratedSkillFile>), SkillError> {
    let manifest_path = source_dir.join("skill.toml");
    if manifest_path.exists() {
        let toml_str = std::fs::read_to_string(&manifest_path)?;
        let manifest: SkillManifest = toml::from_str(&toml_str)?;
        return Ok((manifest, LocalSkillFormat::Native, Vec::new()));
    }

    if openclaw_compat::detect_skillmd(source_dir) {
        let converted = openclaw_compat::convert_skillmd(source_dir)?;
        let manifest_toml = toml::to_string_pretty(&converted.manifest)
            .map_err(|err| SkillError::InvalidManifest(format!("TOML serialize: {err}")))?;
        return Ok((
            converted.manifest,
            LocalSkillFormat::SkillMd,
            vec![GeneratedSkillFile {
                relative_path: PathBuf::from("skill.toml"),
                contents: manifest_toml.into_bytes(),
            }],
        ));
    }

    if openclaw_compat::detect_openclaw_skill(source_dir) {
        let manifest = openclaw_compat::convert_openclaw_skill(source_dir)?;
        let manifest_toml = toml::to_string_pretty(&manifest)
            .map_err(|err| SkillError::InvalidManifest(format!("TOML serialize: {err}")))?;
        return Ok((
            manifest,
            LocalSkillFormat::OpenClaw,
            vec![GeneratedSkillFile {
                relative_path: PathBuf::from("skill.toml"),
                contents: manifest_toml.into_bytes(),
            }],
        ));
    }

    Err(SkillError::NotFound(format!(
        "No skill.toml, SKILL.md, or OpenClaw package.json found in {}",
        source_dir.display()
    )))
}

fn validate_manifest(manifest: &SkillManifest, source_dir: &Path) -> Result<(), SkillError> {
    if manifest.skill.name.trim().is_empty() {
        return Err(SkillError::InvalidManifest(
            "Skill name must not be empty".to_string(),
        ));
    }
    if manifest.skill.version.trim().is_empty() {
        return Err(SkillError::InvalidManifest(
            "Skill version must not be empty".to_string(),
        ));
    }

    let mut seen_tools = HashSet::new();
    for tool in &manifest.tools.provided {
        if tool.name.trim().is_empty() {
            return Err(SkillError::InvalidManifest(
                "Skill tools must have a name".to_string(),
            ));
        }
        if !seen_tools.insert(tool.name.clone()) {
            return Err(SkillError::InvalidManifest(format!(
                "Duplicate skill tool name: {}",
                tool.name
            )));
        }
    }

    match manifest.runtime.runtime_type {
        SkillRuntime::Python | SkillRuntime::Node | SkillRuntime::Wasm => {
            if manifest.runtime.entry.trim().is_empty() {
                return Err(SkillError::InvalidManifest(format!(
                    "Runtime {:?} requires an entry path",
                    manifest.runtime.runtime_type
                )));
            }
            let entry_path = source_dir.join(&manifest.runtime.entry);
            if !entry_path.exists() {
                return Err(SkillError::InvalidManifest(format!(
                    "Skill entry not found: {}",
                    entry_path.display()
                )));
            }
        }
        SkillRuntime::Builtin | SkillRuntime::PromptOnly => {}
    }

    Ok(())
}

fn should_include_entry(source_dir: &Path, output_dir: &Path, entry_path: &Path) -> bool {
    if entry_path == source_dir {
        return true;
    }
    if entry_path.starts_with(output_dir) {
        return false;
    }

    let Ok(relative_path) = entry_path.strip_prefix(source_dir) else {
        return false;
    };

    for component in relative_path.components() {
        let name = component.as_os_str().to_string_lossy();
        if matches!(
            name.as_ref(),
            ".git"
                | ".github"
                | "node_modules"
                | "target"
                | "__pycache__"
                | ".pytest_cache"
                | ".venv"
                | "venv"
        ) {
            return false;
        }
    }

    relative_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name != ".DS_Store")
        .unwrap_or(true)
}

fn normalize_zip_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn zip_error(err: zip::result::ZipError) -> SkillError {
    SkillError::InvalidManifest(format!("Zip error: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_prepare_local_skill_from_skillmd_generates_manifest() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("SKILL.md"),
            r#"---
name: Prompt Writer
description: Writes polished copy
---
# Instructions

Write clearly.
"#,
        )
        .unwrap();

        let prepared = prepare_local_skill(dir.path()).unwrap();
        assert_eq!(prepared.format, LocalSkillFormat::SkillMd);
        assert_eq!(prepared.manifest.skill.name, "Prompt Writer");
        assert!(prepared
            .generated_files
            .iter()
            .any(|file| file.relative_path == Path::new("skill.toml")));
    }

    #[test]
    fn test_prepare_local_skill_from_openclaw_generates_manifest() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{
  "name": "test-skill",
  "version": "1.2.3",
  "description": "OpenClaw skill"
}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("index.js"), "console.log('ok');").unwrap();

        let prepared = prepare_local_skill(dir.path()).unwrap();
        assert_eq!(prepared.format, LocalSkillFormat::OpenClaw);
        assert_eq!(prepared.manifest.skill.name, "test-skill");
        assert!(prepared
            .generated_files
            .iter()
            .any(|file| file.relative_path == Path::new("skill.toml")));
    }

    #[test]
    fn test_package_prepared_skill_writes_generated_manifest() {
        let dir = TempDir::new().unwrap();
        let output_dir = dir.path().join("dist");
        std::fs::write(
            dir.path().join("SKILL.md"),
            r#"---
name: Prompt Writer
description: Writes polished copy
---
# Instructions

Write clearly.
"#,
        )
        .unwrap();

        let prepared = prepare_local_skill(dir.path()).unwrap();
        let packaged = package_prepared_skill(&prepared, &output_dir).unwrap();

        assert!(packaged.archive_path.exists());
        let file = std::fs::File::open(&packaged.archive_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert!(archive.by_name("skill.toml").is_ok());
        assert!(archive.by_name("SKILL.md").is_ok());
    }
}
