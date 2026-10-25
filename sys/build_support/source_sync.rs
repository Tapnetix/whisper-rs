//! Filesystem helpers shared by the build script and its native-free tests.
//!
//! The vendored CMake tree is both an input and a mutable CMake source tree
//! (configure_file writes into it). Keep Cargo's watcher on the pristine
//! checkout while synchronizing a complete copy into OUT_DIR before CMake.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub copied_files: usize,
    pub removed_files: usize,
    pub removed_directories: usize,
}

impl SyncReport {
    pub fn changed(&self) -> bool {
        self.copied_files != 0 || self.removed_files != 0 || self.removed_directories != 0
    }
}

/// Return every directory and regular file in deterministic order.
///
/// Emitting both files and directories as Cargo inputs catches content edits
/// as well as additions and removals from nested source directories.
pub fn watched_paths(root: &Path) -> io::Result<Vec<PathBuf>> {
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "native source root must be a real directory",
        ));
    }

    let mut paths = Vec::new();
    collect_paths(root, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_paths(root: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    paths.push(root.to_path_buf());
    let mut entries = fs::read_dir(root)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("native source tree contains unsupported symlink: {}", path.display()),
            ));
        } else if metadata.is_dir() {
            collect_paths(&path, paths)?;
        } else if metadata.is_file() {
            paths.push(path);
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("native source tree contains unsupported entry: {}", path.display()),
            ));
        }
    }

    Ok(())
}

/// Synchronize a source directory into a build-owned directory.
///
/// File contents are compared before copying, so a no-change build preserves
/// destination mtimes. Missing source files are removed from the destination,
/// preventing stale C++ or headers from surviving an incremental build.
pub fn sync_source_tree(source: &Path, destination: &Path) -> io::Result<SyncReport> {
    let source_metadata = fs::symlink_metadata(source)?;
    if !source_metadata.is_dir() || source_metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "native source root must be a real directory",
        ));
    }

    let mut report = SyncReport::default();
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => remove_entry(destination, &mut report)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::create_dir_all(destination)?;
    sync_directory(source, destination, &mut report)?;
    Ok(report)
}

fn sync_directory(source: &Path, destination: &Path, report: &mut SyncReport) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    let mut source_entries = fs::read_dir(source)?.collect::<Result<Vec<_>, _>>()?;
    source_entries.sort_by_key(|entry| entry.file_name());

    for source_entry in &source_entries {
        let source_path = source_entry.path();
        let destination_path = destination.join(source_entry.file_name());
        let source_metadata = fs::symlink_metadata(&source_path)?;
        if source_metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("native source tree contains unsupported symlink: {}", source_path.display()),
            ));
        }

        if source_metadata.is_dir() {
            match fs::symlink_metadata(&destination_path) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => remove_entry(&destination_path, report)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            sync_directory(&source_path, &destination_path, report)?;
        } else if source_metadata.is_file() {
            let destination_matches = match fs::symlink_metadata(&destination_path) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    files_equal(&source_path, &destination_path)?
                }
                Ok(_) => {
                    // Remove a former directory or symlink without following it before copying.
                    remove_entry(&destination_path, report)?;
                    false
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => return Err(error),
            };

            if !destination_matches {
                fs::copy(&source_path, &destination_path)?;
                report.copied_files += 1;
            }
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("native source tree contains unsupported entry: {}", source_path.display()),
            ));
        }
    }

    let source_names = source_entries
        .iter()
        .map(|entry| entry.file_name())
        .collect::<std::collections::HashSet<_>>();
    let mut destination_entries = fs::read_dir(destination)?.collect::<Result<Vec<_>, _>>()?;
    destination_entries.sort_by_key(|entry| entry.file_name());
    for destination_entry in destination_entries {
        if !source_names.contains(&destination_entry.file_name()) {
            remove_entry(&destination_entry.path(), report)?;
        }
    }

    Ok(())
}

fn remove_entry(path: &Path, report: &mut SyncReport) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        let mut entries = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            remove_entry(&entry.path(), report)?;
        }
        fs::remove_dir(path)?;
        report.removed_directories += 1;
    } else {
        fs::remove_file(path)?;
        report.removed_files += 1;
    }
    Ok(())
}

fn files_equal(left: &Path, right: &Path) -> io::Result<bool> {
    let left_metadata = fs::metadata(left)?;
    let right_metadata = fs::metadata(right)?;
    if left_metadata.len() != right_metadata.len() {
        return Ok(false);
    }

    let mut left_file = File::open(left)?;
    let mut right_file = File::open(right)?;
    let mut left_buffer = [0_u8; 16 * 1024];
    let mut right_buffer = [0_u8; 16 * 1024];
    loop {
        let left_count = left_file.read(&mut left_buffer)?;
        let right_count = right_file.read(&mut right_buffer)?;
        if left_count != right_count || left_buffer[..left_count] != right_buffer[..right_count] {
            return Ok(false);
        }
        if left_count == 0 {
            return Ok(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestTree(PathBuf);

    impl TestTree {
        fn new() -> Self {
            static NEXT_TEST_TREE_ID: AtomicU64 = AtomicU64::new(0);
            loop {
                let nonce = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock before Unix epoch")
                    .as_nanos();
                let sequence = NEXT_TEST_TREE_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "whisper-source-sync-{}-{nonce}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create temporary source-sync fixture: {error}"),
                }
            }
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.0.join(relative)
        }
    }

    impl Drop for TestTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, contents: &[u8]) {
        fs::create_dir_all(path.parent().expect("fixture path has parent")).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn edits_cpp_and_header_and_removes_stale_copied_sources() {
        let tree = TestTree::new();
        let source = tree.path("source");
        let destination = tree.path("out/whisper.cpp");
        write(&source.join("src/whisper.cpp"), b"cpp-v1");
        write(&source.join("include/whisper.h"), b"header-v1");
        write(&source.join("src/obsolete.cpp"), b"remove-me");

        let first = sync_source_tree(&source, &destination).unwrap();
        assert_eq!(first.copied_files, 3);
        assert_eq!(fs::read(destination.join("src/whisper.cpp")).unwrap(), b"cpp-v1");
        assert_eq!(fs::read(destination.join("include/whisper.h")).unwrap(), b"header-v1");

        write(&source.join("src/whisper.cpp"), b"cpp-v2");
        write(&source.join("include/whisper.h"), b"header-v2");
        fs::remove_file(source.join("src/obsolete.cpp")).unwrap();
        write(&source.join("include/new-header.h"), b"new-header");

        let update = sync_source_tree(&source, &destination).unwrap();
        assert!(update.changed());
        assert_eq!(update.copied_files, 3);
        assert_eq!(update.removed_files, 1);
        assert!(!destination.join("src/obsolete.cpp").exists());
        assert_eq!(fs::read(destination.join("src/whisper.cpp")).unwrap(), b"cpp-v2");
        assert_eq!(fs::read(destination.join("include/whisper.h")).unwrap(), b"header-v2");
        assert_eq!(fs::read(destination.join("include/new-header.h")).unwrap(), b"new-header");
    }

    #[test]
    fn unchanged_tree_is_a_noop_and_keeps_destination_mtimes() {
        let tree = TestTree::new();
        let source = tree.path("source");
        let destination = tree.path("out/whisper.cpp");
        write(&source.join("src/whisper.cpp"), b"stable");
        write(&source.join("include/whisper.h"), b"stable-header");
        sync_source_tree(&source, &destination).unwrap();
        let cpp_before = fs::metadata(destination.join("src/whisper.cpp")).unwrap().modified().unwrap();
        let header_before = fs::metadata(destination.join("include/whisper.h")).unwrap().modified().unwrap();

        let unchanged = sync_source_tree(&source, &destination).unwrap();
        assert_eq!(unchanged, SyncReport::default());
        let repeated = sync_source_tree(&source, &destination).unwrap();
        assert_eq!(repeated, SyncReport::default());
        assert_eq!(fs::metadata(destination.join("src/whisper.cpp")).unwrap().modified().unwrap(), cpp_before);
        assert_eq!(fs::metadata(destination.join("include/whisper.h")).unwrap().modified().unwrap(), header_before);
    }

    #[test]
    fn source_entry_transitions_between_directory_and_regular_file() {
        let tree = TestTree::new();
        let source = tree.path("source");
        let destination = tree.path("out/whisper.cpp");
        write(&source.join("transition/old.cpp"), b"former directory");
        sync_source_tree(&source, &destination).unwrap();

        fs::remove_dir_all(source.join("transition")).unwrap();
        write(&source.join("transition"), b"now a regular file");
        let directory_to_file = sync_source_tree(&source, &destination).unwrap();
        assert!(destination.join("transition").is_file());
        assert_eq!(fs::read(destination.join("transition")).unwrap(), b"now a regular file");
        assert_eq!(directory_to_file.copied_files, 1);
        assert_eq!(directory_to_file.removed_files, 1);
        assert_eq!(directory_to_file.removed_directories, 1);

        fs::remove_file(source.join("transition")).unwrap();
        write(&source.join("transition/new.cpp"), b"now a directory");
        let file_to_directory = sync_source_tree(&source, &destination).unwrap();
        assert!(destination.join("transition").is_dir());
        assert_eq!(fs::read(destination.join("transition/new.cpp")).unwrap(), b"now a directory");
        assert_eq!(file_to_directory.copied_files, 1);
        assert_eq!(file_to_directory.removed_files, 1);
        assert_eq!(file_to_directory.removed_directories, 0);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_destination_symlink_is_unlinked_without_following_its_target() {
        use std::os::unix::fs::symlink;

        let tree = TestTree::new();
        let source = tree.path("source");
        let destination = tree.path("out/whisper.cpp");
        write(&source.join("generated.json"), b"source data");
        fs::create_dir_all(&destination).unwrap();
        let missing_external = tree.path("outside/not-created.json");
        symlink(&missing_external, destination.join("generated.json")).unwrap();

        let report = sync_source_tree(&source, &destination).unwrap();
        assert_eq!(fs::read(destination.join("generated.json")).unwrap(), b"source data");
        assert!(fs::symlink_metadata(destination.join("generated.json")).unwrap().is_file());
        assert!(!missing_external.exists());
        assert_eq!(report.copied_files, 1);
        assert_eq!(report.removed_files, 1);
    }

    #[test]
    fn stale_files_and_directories_are_removed() {
        let tree = TestTree::new();
        let source = tree.path("source");
        let destination = tree.path("out/whisper.cpp");
        write(&source.join("CMakeLists.txt"), b"source");
        fs::create_dir_all(destination.join("stale/deep")).unwrap();
        write(&destination.join("stale/deep/orphan.cpp"), b"orphan");
        write(&destination.join("stale-root.h"), b"orphan");

        let report = sync_source_tree(&source, &destination).unwrap();
        assert_eq!(report.removed_files, 2);
        assert_eq!(report.removed_directories, 2);
        assert!(!destination.join("stale").exists());
        assert!(!destination.join("stale-root.h").exists());
    }

    #[test]
    fn watched_paths_cover_nested_files_and_directory_membership() {
        let tree = TestTree::new();
        let root = tree.path("whisper.cpp");
        write(&root.join("src/whisper.cpp"), b"cpp");
        write(&root.join("include/whisper.h"), b"header");
        let paths = watched_paths(Path::new(&root)).unwrap();
        assert!(paths.contains(&root));
        assert!(paths.contains(&root.join("src")));
        assert!(paths.contains(&root.join("include")));
        assert!(paths.contains(&root.join("src/whisper.cpp")));
        assert!(paths.contains(&root.join("include/whisper.h")));
    }
}
