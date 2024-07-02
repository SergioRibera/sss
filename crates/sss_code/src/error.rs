use thiserror::Error;

#[derive(Error, Debug)]
#[error(transparent)]
pub enum CodeScreenshot {
    #[error("The expected format for {0} is {1}")]
    InvalidFormat(&'static str, &'static str),
    #[error("Generation Image")]
    GenerationImage(#[from] sss_lib::error::ImagenGeneration),
    Directories(#[from] Configuration),
    Syntect(#[from] syntect::Error),
    SyntectLoading(#[from] syntect::LoadingError),
    VimTheme(#[from] VimTheme),
}

#[derive(Debug, Error)]
#[error(transparent)]
pub enum Configuration {
    Deserialization(#[from] toml::de::Error),
    #[error("Invalid Home directory path from operating system")]
    InvalidHome,
    #[error("Not found `{0}` param in configuration")]
    ParamNotFound(String),
}

#[derive(Debug, Error)]
#[error(transparent)]
pub enum VimTheme {
    #[error("Not found `{0}` param in `vim theme` argument")]
    ParamNotFound(String),
}
