//! Single-writer process lock for mutable state operations (INV-012).

use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// A process-level exclusive lock protecting mutable pkg state from concurrent modifications.
#[derive(Debug)]
pub struct ProcessLock {
    file: File,
    lock_path: PathBuf,
}

impl ProcessLock {
    /// Attempts to acquire an exclusive non-blocking advisory file lock.
    pub fn acquire(lock_path: &Path) -> Result<Self> {
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;

        let fd = file.as_raw_fd();
        // Advisory exclusive non-blocking lock
        let ret = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };

        if ret != 0 {
            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if errno == libc::EWOULDBLOCK || errno == libc::EAGAIN {
                return Err(Error::LockError(format!(
                    "Another pkg process is currently performing mutations (lock held at {})",
                    lock_path.display()
                )));
            }
            return Err(Error::LockError(format!(
                "Failed to acquire process lock at {}: error code {}",
                lock_path.display(),
                errno
            )));
        }

        Ok(Self {
            file,
            lock_path: lock_path.to_path_buf(),
        })
    }

    /// Returns the path to the acquired lock file.
    #[must_use]
    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }
}

impl Drop for ProcessLock {
    fn drop(&mut self) {
        let fd = self.file.as_raw_fd();
        let _ = unsafe { libc::flock(fd, libc::LOCK_UN) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_lock_mutual_exclusion() {
        let temp = tempfile::tempdir().unwrap();
        let lock_path = temp.path().join("test.lock");

        let lock1 = ProcessLock::acquire(&lock_path).unwrap();
        // Second lock attempt must fail
        let lock2_res = ProcessLock::acquire(&lock_path);
        assert!(lock2_res.is_err());

        // Release first lock
        drop(lock1);

        // Now second attempt must succeed
        let lock3 = ProcessLock::acquire(&lock_path);
        assert!(lock3.is_ok());
    }
}
