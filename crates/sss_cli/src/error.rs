use thiserror::Error;

#[derive(Error, Debug)]
#[error(transparent)]
pub enum SSScreenshot {
    #[error("{0}")]
    Custom(String),
    #[error("Generation Image")]
    GenerationImage(#[from] sss_lib::error::ImagenGeneration),
    Directories(#[from] Configuration),
    #[cfg(target_os = "linux")]
    Wayshot(#[from] libwayshot::Error),
}

#[derive(Debug, Error)]
#[error(transparent)]
pub enum Configuration {
    Deserialization(#[from] toml::de::Error),
    #[error("Invalid Home directory path from operating system")]
    InvalidHome,
}
