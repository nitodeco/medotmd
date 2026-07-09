use std::error::Error;

pub type CliResult<T> = Result<T, Box<dyn Error>>;
