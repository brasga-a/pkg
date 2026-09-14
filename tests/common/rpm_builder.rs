#![allow(dead_code, unreachable_pub)]

use rpm::{Dependency, FileOptions, PackageBuilder, Scriptlet};
use std::path::Path;

/// Options for building a synthetic RPM fixture package for tests.
pub struct RpmPackageBuilder {
    name: String,
    version: String,
    release: String,
    architecture: String,
    license: String,
    summary: String,
    description: Option<String>,
    requires: Vec<String>,
    provides: Vec<String>,
    conflicts: Vec<String>,
    scripts: Vec<(String, String)>,
    files: Vec<(String, Vec<u8>, u32)>,
}

impl RpmPackageBuilder {
    /// Starts a new builder with the specified package name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: "1.0.0".to_string(),
            release: "1.fc40".to_string(),
            architecture: "x86_64".to_string(),
            license: "MIT".to_string(),
            summary: "Synthetic test RPM package".to_string(),
            description: None,
            requires: Vec::new(),
            provides: Vec::new(),
            conflicts: Vec::new(),
            scripts: Vec::new(),
            files: Vec::new(),
        }
    }

    /// Sets the package version.
    pub fn version(mut self, ver: impl Into<String>) -> Self {
        self.version = ver.into();
        self
    }

    /// Sets the package release.
    pub fn release(mut self, rel: impl Into<String>) -> Self {
        self.release = rel.into();
        self
    }

    /// Sets the package architecture.
    pub fn architecture(mut self, arch: impl Into<String>) -> Self {
        self.architecture = arch.into();
        self
    }

    /// Adds a requirement/dependency.
    pub fn requires(mut self, req: impl Into<String>) -> Self {
        self.requires.push(req.into());
        self
    }

    /// Adds a capability/provides entry.
    pub fn provides(mut self, prov: impl Into<String>) -> Self {
        self.provides.push(prov.into());
        self
    }

    /// Adds a conflict entry.
    pub fn conflicts(mut self, conf: impl Into<String>) -> Self {
        self.conflicts.push(conf.into());
        self
    }

    /// Adds a lifecycle script (e.g. prein, postin, preun, postun).
    pub fn script(mut self, name: impl Into<String>, content: impl Into<String>) -> Self {
        self.scripts.push((name.into(), content.into()));
        self
    }

    /// Adds a file entry to the RPM payload.
    pub fn file(mut self, path: impl Into<String>, content: impl Into<Vec<u8>>, mode: u32) -> Self {
        self.files.push((path.into(), content.into(), mode));
        self
    }

    /// Builds and writes the RPM package to disk.
    pub fn write_to(&self, destination: &Path) -> std::io::Result<()> {
        let mut builder = PackageBuilder::new(
            &self.name,
            &self.version,
            &self.license,
            &self.architecture,
            &self.summary,
        );

        builder.release(&self.release);
        if let Some(desc) = &self.description {
            builder.description(desc);
        }

        for req in &self.requires {
            builder.requires(Dependency::any(req));
        }

        for prov in &self.provides {
            builder.provides(Dependency::any(prov));
        }

        for conf in &self.conflicts {
            builder.conflicts(Dependency::any(conf));
        }

        for (name, content) in &self.scripts {
            match name.as_str() {
                "pre" | "prein" => builder.pre_install_script(Scriptlet::new(content)),
                "post" | "postin" => builder.post_install_script(Scriptlet::new(content)),
                "preun" => builder.pre_uninstall_script(Scriptlet::new(content)),
                "postun" => builder.post_uninstall_script(Scriptlet::new(content)),
                _ => &mut builder,
            };
        }

        for (path, content, mode) in &self.files {
            let clean_path = if path.starts_with('/') {
                path.clone()
            } else {
                format!("/{path}")
            };
            builder
                .with_file_contents(
                    content.clone(),
                    FileOptions::new(clean_path).permissions(*mode as u16),
                )
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }

        let pkg = builder
            .build()
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        pkg.write_file(destination)
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        Ok(())
    }
}
