//! The settings file: `currents/config.toml` in the user's config directory, or wherever
//! `CURRENTS_CONFIG` points.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use currents_core::settings::{self, Settings};
use currents_core::viewer::Store;
use toml_edit::DocumentMut;

const ENV: &str = "CURRENTS_CONFIG";

/// What the file held when the viewer started.
pub(crate) struct Loaded {
    pub(crate) settings: Settings,
    pub(crate) store: FileStore,
    pub(crate) issues: Vec<String>,
}

pub(crate) struct FileStore {
    path: Option<PathBuf>,
    /// The file as read, so a save rewrites only the keys that changed.
    doc: DocumentMut,
    /// The settings the file now stands for, beside any overrides of this run.
    saved: Settings,
    /// Why saving is off; a file that would not read is never overwritten.
    problem: Option<String>,
}

fn default_path() -> Option<PathBuf> {
    std::env::var_os(ENV)
        .map(PathBuf::from)
        .or_else(|| dirs::config_dir().map(|d| d.join("currents").join("config.toml")))
}

/// Reads the settings file, falling back to defaults (and to not saving) when it cannot be used.
pub(crate) fn load() -> Loaded {
    load_from(default_path())
}

fn disabled(path: Option<PathBuf>, problem: String) -> Loaded {
    tracing::warn!(%problem, "settings will not be saved");
    let store = FileStore { path, doc: DocumentMut::new(), saved: Settings::default(), problem: Some(problem) };
    Loaded { settings: Settings::default(), store, issues: Vec::new() }
}

fn load_from(path: Option<PathBuf>) -> Loaded {
    let Some(file) = path.clone() else {
        return disabled(None, format!("No config directory was found; set {ENV} to a file to save settings."));
    };
    let text = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => return disabled(path, format!("{} could not be read: {e}", file.display())),
    };
    let (settings, issues) = match settings::toml::load(&text) {
        Ok(loaded) => loaded,
        Err(e) => {
            let reason = snafu::Report::from_error(e);
            return disabled(path, format!("{} was not loaded ({reason}); fix it to save again.", file.display()));
        },
    };
    let doc = text.parse().unwrap_or_default();
    let issues: Vec<String> = issues.iter().map(ToString::to_string).collect();
    for issue in &issues {
        tracing::warn!(%issue, file = %file.display(), "settings file");
    }
    let store = FileStore { path, doc, saved: settings.clone(), problem: None };
    Loaded { settings, store, issues }
}

impl FileStore {
    /// Treats `settings` as what the file stands for, so overrides of this run are only
    /// written once they are changed again.
    pub(crate) fn assume(&mut self, settings: &Settings) {
        self.saved = settings.clone();
    }

    fn write(path: &Path, text: &str) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)
    }
}

impl Store for FileStore {
    fn save(&mut self, settings: &Settings) -> Result<(), String> {
        let (None, Some(path)) = (&self.problem, &self.path) else { return Ok(()) };
        if settings::toml::write_back(&mut self.doc, &self.saved, settings) {
            Self::write(path, &self.doc.to_string()).map_err(|e| format!("{}: {e}", path.display()))?;
            tracing::debug!(file = %path.display(), "settings saved");
        }
        self.saved = settings.clone();
        Ok(())
    }

    fn location(&self) -> Option<String> {
        self.path.as_ref().map(|p| p.display().to_string())
    }

    fn open(&self) -> Result<(), String> {
        let path = self.path.as_ref().ok_or("there is no settings file")?;
        if !path.exists() {
            Self::write(path, &self.doc.to_string()).map_err(|e| e.to_string())?;
        }
        open::that_detached(path).map_err(|e| e.to_string())
    }

    fn problem(&self) -> Option<String> {
        self.problem.clone()
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("currents-store-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("config.toml")
    }

    #[test]
    fn overrides_of_one_run_are_not_written_until_changed() {
        let path = scratch("overrides");
        let mut loaded = load_from(Some(path.clone()));
        let mut run = loaded.settings.clone();
        run.motion.speed = 2.0;
        loaded.store.assume(&run);
        let mut edited = run.clone();
        edited.render.fps_cap = 30;
        check!(loaded.store.save(&edited).is_ok());
        let text = fs::read_to_string(&path).unwrap();
        check!(text.contains("fps_cap = 30"));
        check!(!text.contains("speed"));
    }

    #[test]
    fn a_file_that_is_not_toml_is_never_overwritten() {
        let path = scratch("broken");
        FileStore::write(&path, "[render\n").unwrap();
        let mut loaded = load_from(Some(path.clone()));
        check!(loaded.store.problem().is_some());
        let mut edited = loaded.settings.clone();
        edited.render.fps_cap = 30;
        check!(loaded.store.save(&edited).is_ok());
        check!(fs::read_to_string(&path).unwrap() == "[render\n");
    }
}
