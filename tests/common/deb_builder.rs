#![allow(dead_code, unreachable_pub)]

use flate2::Compression;
use flate2::write::GzEncoder;
use std::io::Cursor;
use std::path::Path;

/// Options for building a synthetic `.deb` fixture package.
pub struct DebPackageBuilder {
    name: String,
    version: String,
    architecture: String,
    description: Option<String>,
    depends: Option<String>,
    scripts: Vec<(String, String)>,
    files: Vec<(String, Vec<u8>, u32)>, // (relative path, content, mode)
    symlinks: Vec<(String, String)>,    // (relative path, target)
    directories: Vec<String>,
    corrupt_magic: bool,
    corrupt_debian_binary: bool,
}

impl DebPackageBuilder {
    /// Starts a new builder with the specified package name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: "1.0.0".to_string(),
            architecture: "amd64".to_string(),
            description: Some("Synthetic test package".to_string()),
            depends: None,
            scripts: Vec::new(),
            files: Vec::new(),
            symlinks: Vec::new(),
            directories: Vec::new(),
            corrupt_magic: false,
            corrupt_debian_binary: false,
        }
    }

    /// Sets the package version.
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Sets the package architecture.
    pub fn architecture(mut self, arch: impl Into<String>) -> Self {
        self.architecture = arch.into();
        self
    }

    /// Sets package dependencies.
    pub fn depends(mut self, depends: impl Into<String>) -> Self {
        self.depends = Some(depends.into());
        self
    }

    /// Adds a maintainer lifecycle script (e.g. preinst, postinst).
    pub fn script(mut self, name: impl Into<String>, content: impl Into<String>) -> Self {
        self.scripts.push((name.into(), content.into()));
        self
    }

    /// Adds a directory to the data payload.
    pub fn directory(mut self, path: impl Into<String>) -> Self {
        self.directories.push(path.into());
        self
    }

    /// Adds an executable binary or file entry to the data payload.
    pub fn file(mut self, path: impl Into<String>, content: impl Into<Vec<u8>>, mode: u32) -> Self {
        self.files.push((path.into(), content.into(), mode));
        self
    }

    /// Adds a symlink to the data payload.
    pub fn symlink(mut self, path: impl Into<String>, target: impl Into<String>) -> Self {
        self.symlinks.push((path.into(), target.into()));
        self
    }

    /// Marks the archive to have a corrupt ar magic header.
    pub fn corrupt_magic(mut self) -> Self {
        self.corrupt_magic = true;
        self
    }

    /// Marks the archive to have an invalid debian-binary version.
    pub fn corrupt_debian_binary(mut self) -> Self {
        self.corrupt_debian_binary = true;
        self
    }

    /// Builds and writes the `.deb` archive to the given destination path.
    pub fn write_to(&self, destination: &Path) -> std::io::Result<()> {
        let bytes = self.build_bytes()?;
        std::fs::write(destination, bytes)?;
        Ok(())
    }

    /// Assembles the complete `.deb` container into an in-memory byte buffer.
    pub fn build_bytes(&self) -> std::io::Result<Vec<u8>> {
        if self.corrupt_magic {
            return Ok(b"!<corrupt-magic>\ntruncated".to_vec());
        }

        // 1. Build control.tar.gz
        let control_tar_gz = self.build_control_tar_gz()?;

        // 2. Build data.tar.gz
        let data_tar_gz = self.build_data_tar_gz()?;

        // 3. Assemble ar archive
        let mut ar_buf = Cursor::new(Vec::new());
        {
            let mut ar = ar::Builder::new(&mut ar_buf);

            // Entry 1: debian-binary
            let deb_binary_content = if self.corrupt_debian_binary {
                b"9.9\n"
            } else {
                b"2.0\n"
            };
            let mut header =
                ar::Header::new(b"debian-binary".to_vec(), deb_binary_content.len() as u64);
            header.set_mode(0o644);
            ar.append(&header, &deb_binary_content[..])?;

            // Entry 2: control.tar.gz
            let mut header =
                ar::Header::new(b"control.tar.gz".to_vec(), control_tar_gz.len() as u64);
            header.set_mode(0o644);
            ar.append(&header, &control_tar_gz[..])?;

            // Entry 3: data.tar.gz
            let mut header = ar::Header::new(b"data.tar.gz".to_vec(), data_tar_gz.len() as u64);
            header.set_mode(0o644);
            ar.append(&header, &data_tar_gz[..])?;
        }

        Ok(ar_buf.into_inner())
    }

    fn build_control_tar_gz(&self) -> std::io::Result<Vec<u8>> {
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut tar = tar::Builder::new(&mut gz);

            // control file content
            let mut control = format!(
                "Package: {}\nVersion: {}\nArchitecture: {}\n",
                self.name, self.version, self.architecture
            );
            if let Some(ref desc) = self.description {
                control.push_str(&format!("Description: {desc}\n"));
            }
            if let Some(ref deps) = self.depends {
                control.push_str(&format!("Depends: {deps}\n"));
            }

            let control_bytes = control.as_bytes();
            let mut header = tar::Header::new_gnu();
            header.set_size(control_bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, "./control", control_bytes)?;

            // Maintainer scripts
            for (script_name, script_content) in &self.scripts {
                let bytes = script_content.as_bytes();
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                tar.append_data(&mut header, format!("./{script_name}"), bytes)?;
            }

            tar.finish()?;
        }
        gz.finish()
    }

    fn build_data_tar_gz(&self) -> std::io::Result<Vec<u8>> {
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut tar = tar::Builder::new(&mut gz);

            // Directories
            for dir in &self.directories {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Directory);
                header.set_size(0);
                header.set_mode(0o755);
                let raw_bytes = header.as_mut_bytes();
                raw_bytes[..100].fill(0);
                let path_bytes = dir.as_bytes();
                let len = path_bytes.len().min(100);
                raw_bytes[..len].copy_from_slice(&path_bytes[..len]);
                header.set_cksum();
                tar.append(&header, &[][..])?;
            }

            // Files
            for (path, content, mode) in &self.files {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Regular);
                header.set_size(content.len() as u64);
                header.set_mode(*mode);
                let raw_bytes = header.as_mut_bytes();
                raw_bytes[..100].fill(0);
                let path_bytes = path.as_bytes();
                let len = path_bytes.len().min(100);
                raw_bytes[..len].copy_from_slice(&path_bytes[..len]);
                header.set_cksum();
                tar.append(&header, &content[..])?;
            }

            // Symlinks
            for (path, target) in &self.symlinks {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                header.set_mode(0o777);
                header.set_link_name(target)?;
                let raw_bytes = header.as_mut_bytes();
                raw_bytes[..100].fill(0);
                let path_bytes = path.as_bytes();
                let len = path_bytes.len().min(100);
                raw_bytes[..len].copy_from_slice(&path_bytes[..len]);
                header.set_cksum();
                tar.append(&header, &[][..])?;
            }

            tar.finish()?;
        }
        gz.finish()
    }
}
