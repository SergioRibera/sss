use thiserror::Error;

#[derive(Error, Debug)]
pub enum CodeScreenshotError {
    #[error("The expected format for {0} is {1}")]
    InvalidFormat(&'static str, &'static str),
    #[error("Generation Image")]
    GenerationImage(#[from] sss_lib::error::ImagenGeneration),
}
