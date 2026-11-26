//! Targets: named receivers in a TOML file, merged with command-line options.
//!
//! ```toml
//! [wall]
//! host = "192.0.2.21"        # or a list: ["192.0.2.21", "192.0.2.22"]; or multicast = true
//! size = "192x64"
//! universe = 1
//! lights_per_universe = 170
//! layout = "rows"            # rows, serpentine-rows, columns, serpentine-columns
//! origin = "top-left"        # corner of the first LED
//! fps = 30
//! priority = 100
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::geometry::{Geometry, GeometryError, Layout, Origin, Size, Wiring};
use crate::sacn::{self, Cid, HeaderError, Source};

pub const DEFAULT_UNIVERSE: u16 = 1;
pub const DEFAULT_LIGHTS_PER_UNIVERSE: u16 = 170;
pub const DEFAULT_FPS: f64 = 30.0;
pub const MAX_FPS: f64 = 1000.0;
pub const DEFAULT_SOURCE_NAME: &str = "pixel-cast";
pub const DEFAULT_MULTICAST_TTL: u32 = 1;

/// One target as written in the file or given on the command line: every field may be missing.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetSpec {
    /// Unicast destinations: host names or IPv4 addresses.
    pub host: Option<Hosts>,
    /// Send to the multicast group of each universe instead of `host`.
    pub multicast: Option<bool>,
    pub port: Option<u16>,
    /// Multicast time to live.
    pub ttl: Option<u32>,
    /// IPv4 address of the interface multicast leaves from.
    pub interface: Option<Ipv4Addr>,
    /// `<width>x<height>`.
    pub size: Option<String>,
    /// First universe.
    pub universe: Option<u16>,
    pub lights_per_universe: Option<u16>,
    /// How LEDs run: `rows`, `serpentine-rows`, `columns`, `serpentine-columns`.
    pub layout: Option<String>,
    /// Corner of the first LED: `top-left`, `top-right`, `bottom-left`, `bottom-right`.
    pub origin: Option<String>,
    pub fps: Option<f64>,
    pub priority: Option<u8>,
    pub source_name: Option<String>,
}

/// One host or several.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Hosts {
    One(String),
    Many(Vec<String>),
}

impl Hosts {
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            Hosts::One(host) => vec![host.clone()],
            Hosts::Many(hosts) => hosts.clone(),
        }
    }
}

impl TargetSpec {
    /// `self` where it says something, `fallback` elsewhere: command-line options over the target file.
    pub fn or(self, fallback: TargetSpec) -> TargetSpec {
        TargetSpec {
            host: self.host.or(fallback.host),
            multicast: self.multicast.or(fallback.multicast),
            port: self.port.or(fallback.port),
            ttl: self.ttl.or(fallback.ttl),
            interface: self.interface.or(fallback.interface),
            size: self.size.or(fallback.size),
            universe: self.universe.or(fallback.universe),
            lights_per_universe: self.lights_per_universe.or(fallback.lights_per_universe),
            layout: self.layout.or(fallback.layout),
            origin: self.origin.or(fallback.origin),
            fps: self.fps.or(fallback.fps),
            priority: self.priority.or(fallback.priority),
            source_name: self.source_name.or(fallback.source_name),
        }
    }

    /// Checks every field and fills in the defaults.
    pub fn resolve(&self, cid: Cid) -> Result<Target, TargetError> {
        let port = self.port.unwrap_or(sacn::PORT);
        let destination = match (self.multicast.unwrap_or(false), &self.host) {
            (true, Some(_)) => return Err(TargetError::HostAndMulticast),
            (true, None) => Destination::Multicast {
                port,
                ttl: self.ttl.unwrap_or(DEFAULT_MULTICAST_TTL),
                interface: self.interface,
            },
            (false, Some(hosts)) => {
                let hosts = hosts.to_vec();
                if hosts.is_empty() || hosts.iter().any(|host| host.trim().is_empty()) {
                    return Err(TargetError::NoDestination);
                }
                Destination::Unicast { hosts, port }
            }
            (false, None) => return Err(TargetError::NoDestination),
        };
        let size: Size = self.size.as_deref().ok_or(TargetError::NoSize)?.parse()?;
        let wiring = Wiring {
            layout: match &self.layout {
                Some(layout) => layout.parse()?,
                None => Layout::default(),
            },
            origin: match &self.origin {
                Some(origin) => origin.parse()?,
                None => Origin::default(),
            },
        };
        let geometry = Geometry::new(
            size,
            self.universe.unwrap_or(DEFAULT_UNIVERSE),
            self.lights_per_universe
                .unwrap_or(DEFAULT_LIGHTS_PER_UNIVERSE),
        )?
        .with_wiring(wiring);
        let fps = self.fps.unwrap_or(DEFAULT_FPS);
        if !(fps > 0.0 && fps <= MAX_FPS) {
            return Err(TargetError::Fps(fps));
        }
        let source = Source::new(
            cid,
            self.source_name.as_deref().unwrap_or(DEFAULT_SOURCE_NAME),
            self.priority.unwrap_or(sacn::DEFAULT_PRIORITY),
        )?;
        Ok(Target {
            destination,
            geometry,
            fps,
            source,
        })
    }
}

/// A target ready to stream to.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub destination: Destination,
    pub geometry: Geometry,
    pub fps: f64,
    pub source: Source,
}

/// Where packets go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    /// Every universe to each host.
    Unicast { hosts: Vec<String>, port: u16 },
    /// Each universe to its own group (239.255.<high>.<low>).
    Multicast {
        port: u16,
        ttl: u32,
        interface: Option<Ipv4Addr>,
    },
}

/// The targets of a file, by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Targets {
    targets: BTreeMap<String, TargetSpec>,
}

impl Targets {
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        Ok(Targets {
            targets: toml::from_str(text)?,
        })
    }

    /// Reads `path`. A missing default file is no error (no targets yet); a missing file asked for by name is.
    pub fn load(path: &Path, required: bool) -> Result<Self, TargetError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Targets::parse(&text).map_err(|error| TargetError::Parse {
                path: path.to_owned(),
                message: error.to_string(),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !required => {
                Ok(Targets::default())
            }
            Err(error) => Err(TargetError::Read {
                path: path.to_owned(),
                message: error.to_string(),
            }),
        }
    }

    pub fn get(&self, name: &str) -> Result<&TargetSpec, TargetError> {
        self.targets.get(name).ok_or_else(|| TargetError::Unknown {
            name: name.to_owned(),
            known: self.names().map(str::to_owned).collect(),
        })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.targets.keys().map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &TargetSpec)> {
        self.targets
            .iter()
            .map(|(name, spec)| (name.as_str(), spec))
    }
}

fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "", "pixel-cast")
}

/// The per-platform target file: `~/.config/pixel-cast/targets.toml` on Linux.
pub fn default_targets_path() -> Option<PathBuf> {
    project_dirs().map(|dirs| dirs.config_dir().join("targets.toml"))
}

/// Where this machine's CID is kept: `~/.local/share/pixel-cast/cid` on Linux.
pub fn default_cid_path() -> Option<PathBuf> {
    project_dirs().map(|dirs| dirs.data_local_dir().join("cid"))
}

/// The CID kept in `path`, created there on first use so that it stays the same on a given machine.
pub fn load_or_create_cid(path: &Path) -> Result<Cid, TargetError> {
    let cid_error = |message: String| TargetError::Cid {
        path: path.to_owned(),
        message,
    };
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .trim()
            .parse()
            .map_err(|error: HeaderError| cid_error(error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let cid = Cid::random();
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|error| cid_error(error.to_string()))?;
            }
            std::fs::write(path, format!("{cid}\n"))
                .map_err(|error| cid_error(error.to_string()))?;
            Ok(cid)
        }
        Err(error) => Err(cid_error(error.to_string())),
    }
}

/// A target that cannot be read or used.
#[derive(Clone, Debug, PartialEq)]
pub enum TargetError {
    Read { path: PathBuf, message: String },
    Parse { path: PathBuf, message: String },
    Unknown { name: String, known: Vec<String> },
    Cid { path: PathBuf, message: String },
    NoDestination,
    HostAndMulticast,
    NoSize,
    Fps(f64),
    Geometry(GeometryError),
    Header(HeaderError),
}

impl From<GeometryError> for TargetError {
    fn from(error: GeometryError) -> Self {
        TargetError::Geometry(error)
    }
}

impl From<HeaderError> for TargetError {
    fn from(error: HeaderError) -> Self {
        TargetError::Header(error)
    }
}

impl fmt::Display for TargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TargetError::Read { path, message } => {
                write!(f, "cannot read {}: {message}", path.display())
            }
            TargetError::Parse { path, message } => write!(f, "{}: {message}", path.display()),
            TargetError::Unknown { name, known } if known.is_empty() => {
                write!(f, "unknown target {name:?}: no target is defined")
            }
            TargetError::Unknown { name, known } => {
                write!(
                    f,
                    "unknown target {name:?}; known targets: {}",
                    known.join(", ")
                )
            }
            TargetError::Cid { path, message } => {
                write!(f, "CID file {}: {message}", path.display())
            }
            TargetError::NoDestination => f.write_str("no destination: give a host, or multicast"),
            TargetError::HostAndMulticast => f.write_str("give a host or multicast, not both"),
            TargetError::NoSize => f.write_str("no size: give one such as 192x64"),
            TargetError::Fps(fps) => write!(
                f,
                "the frame rate must be above 0 and at most {MAX_FPS}: {fps}"
            ),
            TargetError::Geometry(error) => error.fmt(f),
            TargetError::Header(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for TargetError {}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"
        [wall]
        host = "192.0.2.21"
        size = "192x64"
        priority = 120

        [pair]
        host = ["192.0.2.21", "wall.example"]
        port = 6000
        size = "32x8"
        universe = 10
        lights_per_universe = 128
        fps = 44.5
        source_name = "studio"

        [panel]
        host = "192.0.2.30"
        size = "32x8"
        layout = "serpentine-columns"
        origin = "bottom-right"

        [group]
        multicast = true
        ttl = 4
        interface = "192.0.2.1"
        size = "16x16"
    "#;

    fn cid() -> Cid {
        Cid([7; 16])
    }

    #[test]
    fn a_full_file_resolves_with_defaults() {
        let targets = Targets::parse(FILE).unwrap();
        assert_eq!(
            targets.names().collect::<Vec<_>>(),
            ["group", "pair", "panel", "wall"]
        );

        let wall = targets.get("wall").unwrap().resolve(cid()).unwrap();
        assert_eq!(
            wall.destination,
            Destination::Unicast {
                hosts: vec!["192.0.2.21".into()],
                port: 5568
            }
        );
        assert_eq!(wall.geometry.universe_count(), 73);
        assert_eq!(wall.geometry.first_universe(), 1);
        assert_eq!(wall.geometry.lights_per_universe(), 170);
        assert_eq!(wall.fps, 30.0);
        assert_eq!(wall.source.priority(), 120);
        assert_eq!(wall.source.name(), "pixel-cast");
        assert_eq!(wall.source.cid(), cid());

        let pair = targets.get("pair").unwrap().resolve(cid()).unwrap();
        assert_eq!(
            pair.destination,
            Destination::Unicast {
                hosts: vec!["192.0.2.21".into(), "wall.example".into()],
                port: 6000
            }
        );
        assert_eq!(pair.geometry.universes().collect::<Vec<_>>(), [10, 11]);
        assert_eq!(pair.fps, 44.5);
        assert_eq!(pair.source.name(), "studio");

        assert_eq!(wall.geometry.wiring(), Wiring::default());
        let panel = targets.get("panel").unwrap().resolve(cid()).unwrap();
        assert_eq!(
            panel.geometry.wiring(),
            Wiring {
                layout: Layout::SerpentineColumns,
                origin: Origin::BottomRight
            }
        );

        let group = targets.get("group").unwrap().resolve(cid()).unwrap();
        assert_eq!(
            group.destination,
            Destination::Multicast {
                port: 5568,
                ttl: 4,
                interface: Some(Ipv4Addr::new(192, 0, 2, 1))
            }
        );
    }

    #[test]
    fn options_override_the_file() {
        let targets = Targets::parse(FILE).unwrap();
        let options = TargetSpec {
            priority: Some(150),
            fps: Some(10.0),
            ..TargetSpec::default()
        };
        let target = options
            .or(targets.get("wall").unwrap().clone())
            .resolve(cid())
            .unwrap();
        assert_eq!(target.source.priority(), 150);
        assert_eq!(target.fps, 10.0);
        assert_eq!(target.geometry.size().to_string(), "192x64");
    }

    #[test]
    fn options_alone_make_a_target() {
        let options = TargetSpec {
            host: Some(Hosts::One("192.0.2.50".into())),
            size: Some("8x8".into()),
            ..TargetSpec::default()
        };
        assert!(options.resolve(cid()).is_ok());
    }

    #[test]
    fn mistakes_are_reported() {
        let resolve = |text: &str| {
            Targets::parse(text)
                .unwrap()
                .get("t")
                .unwrap()
                .resolve(cid())
        };
        assert_eq!(
            resolve("[t]\nsize = \"8x8\""),
            Err(TargetError::NoDestination)
        );
        assert_eq!(
            resolve("[t]\nhost = []\nsize = \"8x8\""),
            Err(TargetError::NoDestination)
        );
        assert_eq!(
            resolve("[t]\nhost = \"h\"\nmulticast = true\nsize = \"8x8\""),
            Err(TargetError::HostAndMulticast)
        );
        assert_eq!(resolve("[t]\nhost = \"h\""), Err(TargetError::NoSize));
        assert!(matches!(
            resolve("[t]\nhost = \"h\"\nsize = \"8 by 8\""),
            Err(TargetError::Geometry(_))
        ));
        assert_eq!(
            resolve("[t]\nhost = \"h\"\nsize = \"8x8\"\nfps = 0"),
            Err(TargetError::Fps(0.0))
        );
        assert!(matches!(
            resolve("[t]\nhost = \"h\"\nsize = \"8x8\"\npriority = 201"),
            Err(TargetError::Header(HeaderError::Priority(201)))
        ));
        assert!(matches!(
            resolve("[t]\nhost = \"h\"\nsize = \"8x8\"\nlayout = \"zigzag\""),
            Err(TargetError::Geometry(GeometryError::Layout(_)))
        ));
        assert!(matches!(
            resolve("[t]\nhost = \"h\"\nsize = \"8x8\"\norigin = \"centre\""),
            Err(TargetError::Geometry(GeometryError::Origin(_)))
        ));
        // unknown keys are typos, not silently ignored
        assert!(Targets::parse("[t]\nhots = \"h\"").is_err());
        let unknown = Targets::parse(FILE).unwrap().get("nope").unwrap_err();
        assert_eq!(
            unknown.to_string(),
            "unknown target \"nope\"; known targets: group, pair, panel, wall"
        );
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pixel-cast-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_files() {
        let dir = scratch_dir("targets");
        let path = dir.join("targets.toml");
        assert_eq!(Targets::load(&path, false), Ok(Targets::default()));
        assert!(matches!(
            Targets::load(&path, true),
            Err(TargetError::Read { .. })
        ));
    }

    #[test]
    fn the_cid_is_created_once_then_kept() {
        let dir = scratch_dir("cid");
        let path = dir.join("nested").join("cid");
        let first = load_or_create_cid(&path).unwrap();
        assert_eq!(load_or_create_cid(&path).unwrap(), first);
        std::fs::write(&path, "not a cid").unwrap();
        assert!(matches!(
            load_or_create_cid(&path),
            Err(TargetError::Cid { .. })
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
