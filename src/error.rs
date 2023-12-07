use thiserror::Error;


#[derive(Error, Debug)]
pub enum CodeScreenshotError {
    #[error("The range is invalid")]
    InvalidRange,
    #[error("The expected format for {0} is {1}")]
    InvalidFormat(&'static str, &'static str),
}
