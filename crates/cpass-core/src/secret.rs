pub trait SecretSource: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct EnvSecretSource;

impl SecretSource for EnvSecretSource {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }
}
