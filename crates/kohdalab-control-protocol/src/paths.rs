//! Application-owned user directories. Resolving paths performs no I/O.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub const APPLICATIONS: [&str; 5] = ["iv", "trkr", "camera", "shutter", "ple"];

pub fn home_directory() -> Result<PathBuf, String> {
    home_from(
        std::env::var_os("HOME"),
        std::env::var_os("USERPROFILE"),
        cfg!(windows),
    )
}

fn home_from(
    home: Option<OsString>,
    profile: Option<OsString>,
    windows: bool,
) -> Result<PathBuf, String> {
    let selected = if windows {
        profile.or(home)
    } else {
        home.or(profile)
    };
    selected
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "User home directory is unavailable".into())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPaths {
    root: PathBuf,
}

impl AppPaths {
    pub fn for_app(application: &str) -> Result<Self, String> {
        Self::from_root(home_directory()?.join(".kohdalab"), application)
    }

    /// `root` is the suite's `.kohdalab` directory, not an application directory.
    pub fn from_root(root: impl Into<PathBuf>, application: &str) -> Result<Self, String> {
        let name = application.strip_prefix("kohdalab-").unwrap_or(application);
        if !APPLICATIONS.contains(&name) {
            return Err("Unknown KohdaLab application".into());
        }
        Ok(Self {
            root: root.into().join(name),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }
    pub fn config_file(&self) -> PathBuf {
        self.config_dir().join("config.json")
    }
    pub fn demo_config_file(&self) -> PathBuf {
        self.config_dir().join("demo.json")
    }
    pub fn run_dir(&self) -> PathBuf {
        self.root.join("run")
    }
    pub fn results_dir(&self) -> PathBuf {
        self.root.join("results")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_applications_share_the_same_nested_layout_without_creating_directories() {
        for name in APPLICATIONS {
            let p = AppPaths::from_root("suite", name).unwrap();
            let app = PathBuf::from("suite").join(name);
            assert_eq!(p.config_file(), app.join("config/config.json"));
            assert_eq!(p.demo_config_file(), app.join("config/demo.json"));
            assert_eq!(p.run_dir(), app.join("run"));
            assert_eq!(p.results_dir(), app.join("results"));
            assert_eq!(p.logs_dir(), app.join("logs"));
            assert_eq!(
                AppPaths::from_root("suite", &format!("kohdalab-{name}")).unwrap(),
                p
            );
        }
        for name in ["../iv", "", "iv/config", "unknown"] {
            assert!(AppPaths::from_root("suite", name).is_err());
        }
    }

    #[test]
    fn native_home_precedence_is_independent_of_process_environment() {
        assert_eq!(
            home_from(Some("unix".into()), Some("windows".into()), false).unwrap(),
            PathBuf::from("unix")
        );
        assert_eq!(
            home_from(Some("unix".into()), Some("windows".into()), true).unwrap(),
            PathBuf::from("windows")
        );
        assert!(home_from(None, None, false).is_err());
    }
}
