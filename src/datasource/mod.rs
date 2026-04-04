use std::path::{Path, PathBuf};

pub mod cpu_frequency;
pub mod cpu_usage;
pub mod disk_io;
pub mod disk_smart;
pub mod docker;
pub mod memory_usage;
pub mod network_io;
pub mod nut;
pub mod thermal;
pub mod zfs_arc;
pub mod zfs_dataset;

#[derive(Debug)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

pub trait Reader: Send + Sync {
    fn read_to_string(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl Future<Output = std::io::Result<String>> + Send;

    fn read_dir(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> impl Future<Output = std::io::Result<Vec<DirEntry>>> + Send;
}

pub struct TokioReader {}

impl TokioReader {
    pub fn new() -> Self {
        Self {}
    }
}

impl Reader for TokioReader {
    async fn read_to_string(&self, path: impl AsRef<Path> + Send) -> std::io::Result<String> {
        tokio::fs::read_to_string(path).await
    }

    async fn read_dir(&self, path: impl AsRef<Path> + Send) -> std::io::Result<Vec<DirEntry>> {
        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(path).await?;
        while let Some(entry) = read_dir.next_entry().await? {
            let metadata = entry.metadata().await?;
            entries.push(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path(),
                is_dir: metadata.is_dir(),
            });
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tokio_reader_tests {
    use super::*;
    use std::io::ErrorKind;

    #[tokio::test]
    async fn test_read_to_string() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("hello.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        let reader = TokioReader::new();
        let content = reader.read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn test_read_to_string_not_found() {
        let reader = TokioReader::new();
        let result = reader
            .read_to_string("/tmp/nonexistent_hephaestus_test_file")
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn test_read_dir_lists_files_and_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), "content").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let reader = TokioReader::new();
        let mut entries = reader.read_dir(dir.path()).await.unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].name, "file.txt");
        assert!(!entries[0].is_dir);
        assert_eq!(entries[0].path, dir.path().join("file.txt"));

        assert_eq!(entries[1].name, "subdir");
        assert!(entries[1].is_dir);
        assert_eq!(entries[1].path, dir.path().join("subdir"));
    }

    #[tokio::test]
    async fn test_read_dir_empty_directory() {
        let dir = tempfile::tempdir().unwrap();

        let reader = TokioReader::new();
        let entries = reader.read_dir(dir.path()).await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_read_dir_not_found() {
        let reader = TokioReader::new();
        let result = reader
            .read_dir("/tmp/nonexistent_hephaestus_test_dir")
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn test_read_dir_nested_structure() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("parent");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("child.txt"), "data").unwrap();
        std::fs::create_dir(sub.join("nested")).unwrap();

        let reader = TokioReader::new();

        // read_dir should only return immediate children
        let mut entries = reader.read_dir(&sub).await.unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "child.txt");
        assert!(!entries[0].is_dir);
        assert_eq!(entries[1].name, "nested");
        assert!(entries[1].is_dir);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::datasource::{DirEntry, Reader};
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
        async fn read_to_string(&self, path: impl AsRef<Path> + Send) -> std::io::Result<String> {
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
                        return Err(std::io::Error::new(ErrorKind::Other, "Response not mocked"));
                    }

                    let response = content[*idx].clone();
                    *idx += 1;

                    Ok(response)
                }
            }
        }

        async fn read_dir(&self, path: impl AsRef<Path> + Send) -> std::io::Result<Vec<DirEntry>> {
            let dir = path.as_ref().to_string_lossy().to_string();
            let prefix = if dir.ends_with('/') {
                dir.clone()
            } else {
                format!("{}/", dir)
            };

            let mut seen = HashMap::<String, bool>::new();

            for key in self.data.keys() {
                if let Some(rest) = key.strip_prefix(&prefix) {
                    if let Some(name) = rest.split('/').next() {
                        if name.is_empty() {
                            continue;
                        }
                        // It's a directory if there are more path components after the name
                        let is_dir = rest.len() > name.len();
                        seen.entry(name.to_string())
                            .and_modify(|d| *d = *d || is_dir)
                            .or_insert(is_dir);
                    }
                }
            }

            if seen.is_empty() && !self.data.keys().any(|k| k == &dir) {
                return Err(std::io::Error::new(
                    ErrorKind::NotFound,
                    format!("Directory not found: {}", dir),
                ));
            }

            let mut entries: Vec<DirEntry> = seen
                .into_iter()
                .map(|(name, is_dir)| {
                    let entry_path = Path::new(&dir).join(&name);
                    DirEntry {
                        name,
                        path: entry_path,
                        is_dir,
                    }
                })
                .collect();

            entries.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(entries)
        }
    }
}
