pub trait ParsableError {
    fn into_string(self) -> &'static str
    where
        Self: Sized;
}
