pub trait TransferEngine {
    fn transfer(&self, source: &str, destination: &str) -> Result<()>;
}
