use std::path::Path;

pub mod cpu_frequency;
pub mod memory_usage;
pub mod network_io;
pub mod disk_io;


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
    use std::collections::HashMap;
    use std::path::Path;

    pub struct HardcodedReader {
        data: HashMap<String, String>,
    }

    impl HardcodedReader {
        pub fn new(data: HashMap<String, String>) -> Self {
            Self { data: data.into() }
        }
    }

    impl Reader for HardcodedReader {
        fn read_to_string(
            &self,
            path: impl AsRef<Path> + Send,
        ) -> impl Future<Output = std::io::Result<String>> + Send {
            async move {
                let path = path.as_ref();
                let path = path.to_string_lossy();

                match self.data.get(path.as_ref()) {
                    None => Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("File not found: {}", path),
                    )),
                    Some(content) => Ok(content.clone()),
                }
            }
        }
    }
}
