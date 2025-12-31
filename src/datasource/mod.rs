use std::path::Path;

pub mod cpu_frequency;
pub mod cpu_usage;
pub mod disk_io;
pub mod disk_smart;
pub mod memory_usage;
pub mod network_io;
pub mod nut;

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
    #[allow(clippy::manual_async_fn)]
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
    use std::collections::hash_map::Entry;
    use std::io::ErrorKind;
    use std::path::Path;
    use std::sync::Mutex;

    pub struct HardcodedReader {
        data: HashMap<String, (Mutex<usize>, Vec<String>)>,
    }

    impl HardcodedReader {
        pub fn new() -> Self {
            Self {
                data: HashMap::new(),
            }
        }

        pub fn add_response(&mut self, key: impl Into<String>, value: impl Into<String>) {
            match self.data.entry(key.into()) {
                Entry::Occupied(mut e) => {
                    e.get_mut().1.push(value.into());
                }
                Entry::Vacant(e) => {
                    e.insert((Mutex::new(0), vec![value.into()]));
                }
            }
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
                        ErrorKind::NotFound,
                        format!("File not found: {}", path),
                    )),
                    Some((idx, content)) => {
                        let mut idx = idx.lock().unwrap();
                        if *idx >= content.len() {
                            return Err(std::io::Error::new(
                                ErrorKind::Other,
                                "Response not mocked",
                            ));
                        }

                        let response = content[*idx].clone();
                        *idx += 1;

                        Ok(response)
                    }
                }
            }
        }
    }
}
