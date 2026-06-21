use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("io error: {0}")]
    PlainIo(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("archive extraction failed: {0}")]
    Archive(String),

    #[error("vdf parse error: {0}")]
    Vdf(String),

    #[error("unknown game id: {0}")]
    UnknownGame(String),

    #[error("game not installed / not detected: {0}")]
    GameNotInstalled(String),

    #[error("mod not found: {0}")]
    ModNotFound(String),

    #[error("profile not found: {0}")]
    ProfileNotFound(String),

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
}
