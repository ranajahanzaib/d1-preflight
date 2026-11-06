use std::fmt;
/// Invalid source/configuration rather than an internal execution failure.
#[derive(Debug)]
pub struct InvalidInput(pub String);
impl fmt::Display for InvalidInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for InvalidInput {}
pub fn input(error: impl fmt::Display) -> anyhow::Error {
    InvalidInput(error.to_string()).into()
}
