use std::path::Path;

pub mod memory_usage;

pub trait Reader: Send + Sync {
    fn read_to_string(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl Future<Output = std::io::Result<String>> + Send;
}

pub struct TokioReader {}

impl TokioReader {
    pub fn new() -> Self {
        Self {}
    }
}

impl Reader for TokioReader {
    fn read_to_string(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl Future<Output = std::io::Result<String>> + Send {
        async move { tokio::fs::read_to_string(path).await }
    }
}

#[cfg(test)]
mod tests {
    use crate::datasource::Reader;
    use std::path::Path;

    pub struct HardcodedReader {
        data: String,
    }

    impl HardcodedReader {
        pub fn new(data: impl Into<String>) -> Self {
            Self { data: data.into() }
        }
    }

    impl Reader for HardcodedReader {
        fn read_to_string(
            &self,
            _: impl AsRef<Path>,
        ) -> impl Future<Output = std::io::Result<String>> + Send {
            async move { Ok(self.data.clone()) }
        }
    }
}
