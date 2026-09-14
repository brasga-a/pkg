#![allow(dead_code, unreachable_pub)]

use std::fs::File;
use std::path::Path;

/// Options for building a synthetic ALPM `.pkg.tar.zst` fixture package for tests.
pub struct AlpmPackageBuilder {
    name: String,
    version: String,
    architecture: String,
    description: String,
    depends: Vec<String>,
    provides: Vec<String>,
    conflicts: Vec<String>,
    install_script: Option<String>,
    files: Vec<(String, Vec<u8>, u32)>,
}

impl AlpmPackageBuilder {
    /// Starts a new builder with the specified package name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: "1.0.0-1".to_string(),
            architecture: "x86_64".to_string(),
            description: "Synthetic test ALPM package".to_string(),
            depends: Vec::new(),
            provides: Vec::new(),
            conflicts: Vec::new(),
            install_script: None,
            files: Vec::new(),
        }
    }

    /// Sets the package version.
    pub fn version(mut self, ver: impl Into<String>) -> Self {
        self.version = ver.into();
        self
    }

    /// Sets the package architecture.
    pub fn architecture(mut self, arch: impl Into<String>) -> Self {
        self.architecture = arch.into();
        self
    }

    /// Adds a dependency.
    pub fn depend(mut self, dep: impl Into<String>) -> Self {
        self.depends.push(dep.into());
        self
    }

    /// Adds a provided capability.
    pub fn provides(mut self, prov: impl Into<String>) -> Self {
        self.provides.push(prov.into());
        self
    }

    /// Adds a conflict.
    pub fn conflict(mut self, conf: impl Into<String>) -> Self {
        self.conflicts.push(conf.into());
        self
    }

    /// Adds an .INSTALL maintainer scriptlet.
    pub fn install_script(mut self, script: impl Into<String>) -> Self {
        self.install_script = Some(script.into());
        self
    }

    /// Adds a file entry to the ALPM payload.
    pub fn file(mut self, path: impl Into<String>, content: impl Into<Vec<u8>>, mode: u32) -> Self {
        self.files.push((path.into(), content.into(), mode));
        self
    }

    /// Builds and writes the `.pkg.tar.zst` archive to disk.
    pub fn write_to(&self, destination: &Path) -> std::io::Result<()> {
        let file = File::create(destination)?;
        let enc = zstd::stream::write::Encoder::new(file, 3)?;
        let mut tar = tar::Builder::new(enc.auto_finish());

        // Construct .PKGINFO
        let mut pkginfo = String::new();
        pkginfo.push_str(&format!("pkgname = {}\n", self.name));
        pkginfo.push_str(&format!("pkgver = {}\n", self.version));
        pkginfo.push_str(&format!("pkgdesc = {}\n", self.description));
        pkginfo.push_str(&format!("arch = {}\n", self.architecture));
        pkginfo.push_str("size = 10240\n");

        for dep in &self.depends {
            pkginfo.push_str(&format!("depend = {}\n", dep));
        }
        for prov in &self.provides {
            pkginfo.push_str(&format!("provides = {}\n", prov));
        }
        for conf in &self.conflicts {
            pkginfo.push_str(&format!("conflict = {}\n", conf));
        }

        let mut header = tar::Header::new_gnu();
        header.set_path(".PKGINFO")?;
        header.set_size(pkginfo.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append(&header, pkginfo.as_bytes())?;

        // Optional .INSTALL scriptlet
        if let Some(script) = &self.install_script {
            let mut header = tar::Header::new_gnu();
            header.set_path(".INSTALL")?;
            header.set_size(script.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append(&header, script.as_bytes())?;
        }

        // Payload files
        for (rel_path, content, mode) in &self.files {
            let clean = rel_path.trim_start_matches('/');
            let mut header = tar::Header::new_gnu();
            header.set_path(clean)?;
            header.set_size(content.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            tar.append(&header, content.as_slice())?;
        }

        tar.finish()?;
        Ok(())
    }
}
