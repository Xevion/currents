#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
use snafu::Snafu;

/// WGSL for both passes; `common` is prepended to each before compiling.
#[derive(Clone, Debug)]
pub struct Sources {
    pub common: String,
    pub field: String,
    pub shade: String,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("could not read shader {}", path.display()))]
    Read { path: PathBuf, source: std::io::Error },
}

impl Sources {
    /// The shaders compiled into the binary.
    pub fn embedded() -> Self {
        Self {
            common: include_str!("../../../shaders/common.wgsl").to_owned(),
            field: include_str!("../../../shaders/field.wgsl").to_owned(),
            shade: include_str!("../../../shaders/shade.wgsl").to_owned(),
        }
    }

    /// Reads `common.wgsl`, `field.wgsl` and `shade.wgsl` from `dir`, so edits apply without a rebuild.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_dir(dir: &Path) -> Result<Self, Error> {
        use snafu::ResultExt;

        let read = |name: &str| {
            let path = dir.join(name);
            let src = std::fs::read_to_string(&path).context(ReadSnafu { path: &path })?;
            tracing::debug!(path = %path.display(), bytes = src.len(), "loaded shader");
            Ok(src)
        };
        Ok(Self { common: read("common.wgsl")?, field: read("field.wgsl")?, shade: read("shade.wgsl")? })
    }

    /// The source a pass is compiled from: the shared declarations followed by the pass itself.
    pub(crate) fn assemble(&self, pass: &str) -> String {
        format!("{}\n{pass}", self.common)
    }
}
