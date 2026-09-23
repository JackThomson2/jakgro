use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use super::{Engine, nnue, search};

#[cfg(test)]
mod tests;

/// A failed NNUE configuration change leaves the previous engine usable.
#[derive(Debug)]
pub enum NnueConfigError {
    /// An empty path cannot identify a model.
    EmptyPath,
    /// The model file failed validation or could not be read.
    Load(nnue::LoadError),
    /// A fresh search-cache domain could not be allocated.
    AllocationFailed,
}

impl Display for NnueConfigError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => formatter.write_str("EvalFile requires a non-empty path"),
            Self::Load(error) => Display::fmt(error, formatter),
            Self::AllocationFailed => formatter.write_str("unable to allocate NNUE search caches"),
        }
    }
}

impl Error for NnueConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Load(error) => Some(error),
            _ => None,
        }
    }
}

/// The network shipped inside the executable and used by default.
///
/// `nets/jakgro.nnue` is the published network of the recipe in
/// `tools/nnue_recipe.sh`, continued on the new head's self-play as
/// `docs/tuning/nnue-aggression75.md` records; `nets/jakgro.report.json`
/// records the continuation that produced it.
static EMBEDDED_NETWORK: LazyLock<Arc<nnue::Network>> = LazyLock::new(|| {
    Arc::new(
        nnue::Network::from_bytes(include_bytes!("../../nets/jakgro.nnue"))
            .expect("the embedded network matches the compiled architecture"),
    )
});

/// The `EvalFile` value that selects the embedded network.
pub const EMBEDDED_EVAL_FILE: &str = "<embedded>";

#[derive(Clone, Debug)]
pub(super) struct Configuration {
    network: Option<Arc<nnue::Network>>,
    /// `None` while the embedded network is loaded.
    path: Option<PathBuf>,
    enabled: bool,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            network: Some(Arc::clone(&EMBEDDED_NETWORK)),
            path: None,
            enabled: true,
        }
    }
}

impl Configuration {
    pub(super) fn active_network(&self) -> Option<&nnue::Network> {
        if self.enabled {
            self.network.as_deref()
        } else {
            None
        }
    }
}

impl Engine {
    /// Returns the path of the last successfully loaded network, or `None`
    /// while the embedded network is loaded.
    #[must_use]
    pub fn eval_file(&self) -> Option<&Path> {
        self.neural.path.as_deref()
    }

    /// Reinstates the embedded network, detaching search caches when a file
    /// network was loaded before.
    pub fn load_embedded_eval_file(&mut self) -> Result<(), NnueConfigError> {
        if self.neural.path.is_none() {
            return Ok(());
        }
        self.detach_evaluation_memory()?;
        self.neural.network = Some(Arc::clone(&EMBEDDED_NETWORK));
        self.neural.path = None;
        Ok(())
    }

    /// Whether new searches use the loaded neural evaluator.
    #[must_use]
    pub fn use_nnue(&self) -> bool {
        self.neural.enabled
    }

    /// Loads a validated immutable network in place of the embedded one.
    ///
    /// Successful loads retain position, aggression, resources' sizes and the
    /// enabled flag, but detach search caches from existing engine clones.
    /// Loading or allocation failure changes neither settings nor caches.
    pub fn load_eval_file(&mut self, path: impl AsRef<Path>) -> Result<(), NnueConfigError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(NnueConfigError::EmptyPath);
        }
        let network = Arc::new(nnue::Network::load(path).map_err(NnueConfigError::Load)?);
        let path = path.to_owned();
        self.detach_evaluation_memory()?;
        self.neural.network = Some(network);
        self.neural.path = Some(path);
        Ok(())
    }

    /// Selects the neural (default) or handcrafted evaluator for subsequent
    /// searches.
    ///
    /// Disabling keeps the model for later reuse. A real backend change
    /// detaches the receiving engine's table and carried ordering; existing
    /// clones retain their own evaluator domain.
    pub fn set_use_nnue(&mut self, enabled: bool) -> Result<(), NnueConfigError> {
        if enabled == self.neural.enabled {
            return Ok(());
        }
        self.detach_evaluation_memory()?;
        self.neural.enabled = enabled;
        Ok(())
    }

    fn detach_evaluation_memory(&mut self) -> Result<(), NnueConfigError> {
        let table = search::TranspositionTable::new(self.hash_size_mib())
            .map_err(|_| NnueConfigError::AllocationFailed)?;
        let table = Arc::new(Mutex::new(Arc::new(table)));
        let memory = Arc::new(Mutex::new(search::SearchMemory::default()));
        self.table = table;
        self.memory = memory;
        Ok(())
    }
}
