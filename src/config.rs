
 //! Configuration: routing rules, skip lists, and known-folder resolution.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub name: String,
    /// Absolute path, or a path using tokens like {documents} or "~".
    pub destination: String,
    /// Lowercase extensions including the dot, e.g. ".pdf".
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Files modified more recently than this are skipped (likely still being written).
    #[serde(default = "default_min_age")]
    pub min_age_seconds: u64,
    /// Identical duplicates are moved into this folder inside the destination, never deleted.
    #[serde(default = "default_dup_folder")]
    pub duplicates_folder: String,
    #[serde(default = "default_skip_extensions")]
    pub skip_extensions: Vec<String>,
    #[serde(default = "default_skip_names")]
    pub skip_names: Vec<String>,
    #[serde(default = "default_rules")]
    pub rules: Vec<Rule>,
}

fn default_version() -> u32 {
    1
}

fn default_min_age() -> u64 {
    10
}

fn default_dup_folder() -> String {
    "_Duplicates".to_string()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn default_skip_extensions() -> Vec<String> {
    strings(&[
        ".crdownload",
        ".part",
        ".partial",
        ".opdownload",
        ".download",
        ".tmp",
        ".lnk",
        ".url",
        ".ini",
    ])
}

fn default_skip_names() -> Vec<String> {
    strings(&[".env", "config.local", "desktop.ini", ".DS_Store", "Thumbs.db"])
}

fn rule(name: &str, destination: &str, exts: &[&str]) -> Rule {
    Rule {
        name: name.to_string(),
        destination: destination.to_string(),
        extensions: strings(exts),
    }
}

fn default_rules() -> Vec<Rule> {
    vec![
        rule(
            "Documents",
            "{documents}",
            &[
                ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".txt", ".md",
                ".rtf", ".odt", ".ods", ".odp",
            ],
        ),
        rule(
            "Pictures",
            "{pictures}",
            &[
                ".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".bmp", ".heic", ".ai",
                ".psd",
            ],
        ),
        rule("Videos", "{videos}", &[".mp4", ".mov", ".avi", ".mkv", ".webm", ".wmv"]),
        rule("Music", "{music}", &[".mp3", ".wav", ".flac", ".m4a", ".aac", ".ogg"]),
        rule(
            "Data",
            "{documents}/Data",
            &[".csv", ".tsv", ".json", ".xml", ".sql", ".yaml", ".yml"],
        ),
        rule(
            "Code",
            "{documents}/Code",
            &[
                ".py", ".js", ".ts", ".rs", ".go", ".java", ".c", ".cpp", ".h", ".cs", ".sh",
                ".ps1", ".html", ".css",
            ],
        ),
        rule(
            "Installers",
            "{documents}/Installers",
            &[".exe", ".msi", ".dmg", ".pkg", ".deb", ".rpm", ".iso", ".apk"],
        ),
        rule(
            "Archives",
            "{documents}/Archives",
            &[".zip", ".rar", ".7z", ".tar", ".gz", ".tgz", ".bz2", ".xz"],
        ),
    ]
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: default_version(),
            min_age_seconds: default_min_age(),
            duplicates_folder: default_dup_folder(),
            skip_extensions: default_skip_extensions(),
            skip_names: default_skip_names(),
            rules: default_rules(),
        }
    }
}

impl Config {
    /// Load a config file. A missing file means "use the defaults" (zero configuration).
    pub fn load(path: &Path) -> Result<Config, String> {
        if !path.exists() {
            return Ok(Config::default());
        }

        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read config {}: {e}", path.display()))?;

        let mut cfg: Config = serde_json::from_str(&text)
            .map_err(|e| format!("invalid config {}: {e}", path.display()))?;

        cfg.validate()?;
        Ok(cfg)
    }

    /// Check the config for mistakes and normalize extensions to lowercase.
    pub fn validate(&mut self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!(
                "unsupported config version {} (expected 1)",
                self.version
            ));
        }

        if self.duplicates_folder.trim().is_empty()
            || self.duplicates_folder.contains(['/', '\\'])
        {
            return Err("duplicates_folder must be a single folder name".to_string());
        }

        let mut seen: HashMap<String, String> = HashMap::new();

        for r in &mut self.rules {
            if r.name.trim().is_empty() {
                return Err("every rule needs a non-empty name".to_string());
            }

            if r.destination.trim().is_empty() {
                return Err(format!("rule '{}' has an empty destination", r.name));
            }

            for ext in &mut r.extensions {
                *ext = ext.trim().to_lowercase();

                if !ext.starts_with('.') || ext.len() < 2 {
                    return Err(format!(
                        "rule '{}': extension '{ext}' must start with a dot, like \".pdf\"",
                        r.name
                    ));
                }

                if let Some(prev) = seen.insert(ext.clone(), r.name.clone()) {
                    return Err(format!(
                        "extension '{ext}' appears in both '{prev}' and '{}'",
                        r.name
                    ));
                }
            }
        }

        for e in &mut self.skip_extensions {
            *e = e.trim().to_lowercase();
        }

        Ok(())
    }

    pub fn skip_name_set(&self) -> HashSet<String> {
        self.skip_names.iter().map(|n| n.to_lowercase()).collect()
    }
}

/// Resolves {tokens} such as {documents} using the operating system's known folders,
/// so redirected folders (for example OneDrive on Windows) are handled correctly.
#[derive(Debug, Clone)]
pub struct KnownDirs {
    map: HashMap<String, PathBuf>,
}

impl KnownDirs {
    pub fn detect() -> KnownDirs {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

        let pick = |found: Option<PathBuf>, fallback: &str| {
            found.unwrap_or_else(|| home.join(fallback))
        };

        let mut map = HashMap::new();
        map.insert("documents".to_string(), pick(dirs::document_dir(), "Documents"));
        map.insert("pictures".to_string(), pick(dirs::picture_dir(), "Pictures"));
        map.insert("videos".to_string(), pick(dirs::video_dir(), "Videos"));
        map.insert("music".to_string(), pick(dirs::audio_dir(), "Music"));
        map.insert("downloads".to_string(), pick(dirs::download_dir(), "Downloads"));
        map.insert("desktop".to_string(), pick(dirs::desktop_dir(), "Desktop"));
        map.insert("home".to_string(), home);

        KnownDirs { map }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_map(map: HashMap<String, PathBuf>) -> KnownDirs {
        KnownDirs { map }
    }

    fn get(&self, name: &str) -> Result<String, String> {
        self.map
            .get(name)
            .and_then(|p| p.to_str())
            .map(String::from)
            .ok_or_else(|| format!("unknown or non-UTF-8 folder token '{{{name}}}'"))
    }

    /// Expand "~" and {tokens} into an absolute path.
    pub fn expand(&self, raw: &str) -> Result<PathBuf, String> {
        let raw = raw.trim();

        if raw.is_empty() {
            return Err("empty path".to_string());
        }

        let mut s = raw.to_string();

        if s == "~" || s.starts_with("~/") || s.starts_with("~\\") {
            let home = self.get("home")?;
            s = format!("{home}{}", &s[1..]);
        }

        let mut out = String::new();
        let mut rest = s.as_str();

        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);

            let after = &rest[start + 1..];
            let end = after
                .find('}')
                .ok_or_else(|| format!("unclosed '{{' in path '{raw}'"))?;

            let token = after[..end].to_lowercase();
            out.push_str(&self.get(&token)?);
            rest = &after[end + 1..];
        }

        out.push_str(rest);

        let path = PathBuf::from(out);

        if !path.is_absolute() {
            return Err(format!(
                "'{raw}' is not an absolute path; start it with a token such as {{documents}}"
            ));
        }

        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs_for_test() -> KnownDirs {
        let mut m = HashMap::new();

        m.insert(
            "documents".to_string(),
            std::env::temp_dir().join("tally-test-docs"),
        );

        m.insert(
            "home".to_string(),
            std::env::temp_dir().join("tally-test-home"),
        );

        KnownDirs::from_map(m)
    }

    #[test]
    fn expands_tokens_and_tilde() {
        let k = dirs_for_test();

        assert_eq!(
            k.expand("{documents}/Data").unwrap(),
            std::env::temp_dir()
                .join("tally-test-docs")
                .join("Data")
        );

        assert_eq!(
            k.expand("~/x").unwrap(),
            std::env::temp_dir()
                .join("tally-test-home")
                .join("x")
        );

        assert!(k.expand("{nope}").is_err());
        assert!(k.expand("relative/path").is_err());
        assert!(k.expand("{documents").is_err());
    }

    #[test]
    fn default_config_is_valid() {
        let mut c = Config::default();
        c.validate().unwrap();
    }

    #[test]
    fn rejects_duplicate_extensions() {
        let mut c = Config::default();
        c.rules.push(rule("Dup", "{documents}", &[".PDF"]));
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(serde_json::from_str::<Config>(r#"{"verison": 1}"#).is_err());
    }
}
