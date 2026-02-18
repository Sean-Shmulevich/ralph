//! Reusable PRD templates stored in `~/.ralph/templates/`.
//!
//! Templates are plain markdown files. The first non-empty line starting with `#`
//! is treated as the title; a `> description` blockquote on the next line(s) is
//! the short description shown in `ralph template list`.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// Directory where templates are stored.
fn templates_dir() -> Result<PathBuf> {
    let dir = dirs::home_dir()
        .context("Cannot determine home directory")?
        .join(".ralph")
        .join("templates");
    fs::create_dir_all(&dir).context("Cannot create ~/.ralph/templates/")?;
    Ok(dir)
}

/// Metadata extracted from a template file.
pub struct TemplateMeta {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    #[allow(dead_code)]
    pub path: PathBuf,
}

/// Extract title (first `# ...` line) and description (first `> ...` blockquote)
/// from the beginning of a markdown file.
fn extract_meta(content: &str) -> (Option<String>, Option<String>) {
    let mut title = None;
    let mut desc_lines: Vec<String> = Vec::new();
    let mut past_title = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if past_title && !desc_lines.is_empty() {
                break; // blank line after description ends it
            }
            continue;
        }

        if title.is_none() && trimmed.starts_with('#') {
            title = Some(trimmed.trim_start_matches('#').trim().to_string());
            past_title = true;
            continue;
        }

        if past_title && trimmed.starts_with('>') {
            desc_lines.push(trimmed.trim_start_matches('>').trim().to_string());
            continue;
        }

        if past_title {
            break; // non-blockquote content after title
        }
    }

    let description = if desc_lines.is_empty() {
        None
    } else {
        Some(desc_lines.join(" "))
    };

    (title, description)
}

/// Save a PRD file as a named template.
pub fn save(name: &str, source: &PathBuf) -> Result<()> {
    let dir = templates_dir()?;
    let dest = dir.join(format!("{name}.md"));

    let content = fs::read_to_string(source)
        .with_context(|| format!("Cannot read source PRD: {}", source.display()))?;

    fs::write(&dest, &content)
        .with_context(|| format!("Cannot write template: {}", dest.display()))?;

    let (title, _) = extract_meta(&content);
    let display_title = title.as_deref().unwrap_or(name);
    println!("✅  Saved template '{name}' — {display_title}");
    println!("    {}", dest.display());
    Ok(())
}

/// List all saved templates.
pub fn list(verbose: bool) -> Result<()> {
    let dir = templates_dir()?;
    let mut entries: Vec<TemplateMeta> = Vec::new();

    for entry in fs::read_dir(&dir).context("Cannot read templates directory")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();

        let content = fs::read_to_string(&path).unwrap_or_default();
        let (title, description) = extract_meta(&content);

        entries.push(TemplateMeta {
            name,
            title,
            description,
            path,
        });
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));

    if entries.is_empty() {
        println!("📭  No templates saved yet.");
        println!("    Save one with: ralph template save <name> <prd.md>");
        return Ok(());
    }

    println!("📋  {} template(s) in {}\n", entries.len(), dir.display());

    for t in &entries {
        if verbose {
            let title = t.title.as_deref().unwrap_or("(untitled)");
            let desc = t
                .description
                .as_deref()
                .unwrap_or("(no description)");
            println!("  📄 {}", t.name);
            println!("     {title}");
            println!("     {desc}");
            println!();
        } else {
            match &t.title {
                Some(title) => println!("  📄 {} — {title}", t.name),
                None => println!("  📄 {}", t.name),
            }
        }
    }

    Ok(())
}

/// Get the path to a saved template by name.
pub fn get(name: &str) -> Result<PathBuf> {
    let dir = templates_dir()?;
    let path = dir.join(format!("{name}.md"));
    if !path.exists() {
        // Try to suggest similar names
        let available: Vec<String> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                e.path()
                    .file_stem()
                    .and_then(|s| s.to_str().map(String::from))
            })
            .collect();

        if available.is_empty() {
            anyhow::bail!("Template '{name}' not found. No templates saved yet.\nSave one with: ralph template save <name> <prd.md>");
        } else {
            anyhow::bail!(
                "Template '{name}' not found.\nAvailable: {}",
                available.join(", ")
            );
        }
    }
    Ok(path)
}

/// Remove a saved template.
pub fn remove(name: &str) -> Result<()> {
    let dir = templates_dir()?;
    let path = dir.join(format!("{name}.md"));
    if !path.exists() {
        anyhow::bail!("Template '{name}' not found");
    }
    fs::remove_file(&path).context("Cannot remove template")?;
    println!("🗑️  Removed template '{name}'");
    Ok(())
}

/// Show the full content of a template.
pub fn show(name: &str) -> Result<()> {
    let path = get(name)?;
    let content = fs::read_to_string(&path)?;
    println!("{content}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_meta_parses_title_and_description() {
        let md = "# Code Review\n\n> Automated code review for any codebase.\n> Checks security, performance, and style.\n\n## Tasks\n";
        let (title, desc) = extract_meta(md);
        assert_eq!(title.as_deref(), Some("Code Review"));
        assert_eq!(
            desc.as_deref(),
            Some("Automated code review for any codebase. Checks security, performance, and style.")
        );
    }

    #[test]
    fn extract_meta_handles_title_only() {
        let md = "# Quick Audit\n\n## Tasks\n### T1: Do stuff\n";
        let (title, desc) = extract_meta(md);
        assert_eq!(title.as_deref(), Some("Quick Audit"));
        assert!(desc.is_none());
    }

    #[test]
    fn extract_meta_handles_no_header() {
        let md = "Just some text\nno markdown headers\n";
        let (title, desc) = extract_meta(md);
        assert!(title.is_none());
        assert!(desc.is_none());
    }
}
