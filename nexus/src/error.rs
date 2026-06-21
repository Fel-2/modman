pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("nexus api returned {status}: {body}")]
    Api { status: u16, body: String },

    #[error("invalid nxm link: {0}")]
    BadLink(String),

    #[error("no api key set")]
    NoApiKey,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}
