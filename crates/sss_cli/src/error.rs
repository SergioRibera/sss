use thiserror::Error;

#[derive(Error, Debug)]
#[error(transparent)]
pub enum SSScreenshot {
    #[error("{0}")]
    Custom(String),
    #[error("Generation Image")]
    GenerationImage(#[from] sss_lib::error::ImagenGeneration),
    Directories(#[from] Configuration),
    Capture(#[from] sss_capture::CaptureError),
}

#[derive(Debug, Error)]
#[error(transparent)]
pub enum Configuration {
    Deserialization(#[from] toml::de::Error),
    Io(#[from] std::io::Error),
    #[error("Invalid Home directory path from operating system")]
    InvalidHome,
}
