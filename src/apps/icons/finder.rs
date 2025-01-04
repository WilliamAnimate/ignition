use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fmt::{Display, Formatter};
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use dirs::home_dir;
use tracing::{debug, info, warn};
use xdg::BaseDirectories;
pub struct IconFinder {
    dirs: Vec<CachedDir>,
}

impl IconFinder {
    pub fn new() -> IconFinder {
        warn!("Creating IconFinder, this means that we are going to be looking for icons on your system");
        warn!("This may take a while.");
        IconFinder {
            dirs: icon_theme_base_paths()
                .into_iter()
                .map(|v| {
                    debug!("SEARCHING IN {v:?}");
                    CachedDir::visit(&v).unwrap()
                })
                .collect(),
        }
    }

    fn start_find(
        dir: &mut CachedDir,
        icon_name: &str,
        out: &mut Vec<IconLocation>,
        step: usize,
    ) -> u32 {
        match step {
            0 => {
                if let Some(subdir) = dir.join("hicolor") {
                    return subdir.find(
                        icon_name,
                        out,
                        true,
                        IconDescriptor {
                            theme: Some("hicolor".to_string()),
                            ..IconDescriptor::default()
                        },
                    );
                }
            }
            1 => {
                if let Some(subdir) = dir.join("default") {
                    return subdir.find(
                        icon_name,
                        out,
                        true,
                        IconDescriptor {
                            theme: Some("default".to_string()),
                            ..IconDescriptor::default()
                        },
                    );
                }
            }
            2 => {
                return dir.find(icon_name, out, false, IconDescriptor::default());
            }
            3 => {
                return dir.find(icon_name, out, true, IconDescriptor::default());
            }
            _ => {}
        }

        0
    }
    pub fn find(&mut self, icon_name: &str) -> Vec<IconLocation> {
        let mut out = Vec::new();
        for i in 0..4 {
            if i == 3 {
                warn!("We could not find icon \"{icon_name}\" by simple means.");
                warn!("Scanning the entire tree. (this may take a while)");
            }
            let mut found_any = false;
            for dir in &mut self.dirs {
                found_any |= Self::start_find(dir, icon_name, &mut out, i) > 0;
            }

            if found_any {
                break;
            }
        }

        let any_exact_matches = out.iter().any(|v| v.file_name == icon_name);
        if any_exact_matches {
            out.retain(|location| location.file_name == icon_name);
        } else {
            warn!("Did not find exact match!!");
        }

        out
    }
}

pub struct CachedDir {
    path: PathBuf,
    name: String,
    entries: Option<Vec<CachedDirEntry>>,
}

impl CachedDir {
    pub fn visit(path: &Path) -> io::Result<CachedDir> {
        assert!(path.is_dir());
        Ok(CachedDir {
            path: path.to_path_buf(),
            name: to_string(path.file_name().unwrap()),
            entries: None,
        })
    }

    fn resolve_entries(path: PathBuf) -> io::Result<Vec<CachedDirEntry>> {
        let dir = path.read_dir()?;
        let mut entries = Vec::new();
        for entry in dir {
            let entry = entry?;
            let Some(dir_entry) = CachedDirEntry::visit(&entry.path())? else {
                continue;
            };
            entries.push(dir_entry);
        }

        Ok(entries)
    }

    pub fn entries(&mut self) -> &mut Vec<CachedDirEntry> {
        let buf = self.path.clone();
        self.entries
            .get_or_insert_with(|| Self::resolve_entries(buf).unwrap())
    }
    pub fn files(&mut self) -> Vec<&String> {
        self.entries()
            .iter()
            .flat_map(|entry| {
                let CachedDirEntry::File(file) = entry else {
                    return None;
                };

                Some(file)
            })
            .collect()
    }
    pub fn dirs(&mut self) -> Vec<&mut CachedDir> {
        self.entries()
            .iter_mut()
            .flat_map(|entry| {
                let CachedDirEntry::Directory(dir) = entry else {
                    return None;
                };

                Some(&mut **dir)
            })
            .collect()
    }

    pub fn join(&mut self, name: &str) -> Option<&mut CachedDir> {
        for entry in self.entries() {
            let CachedDirEntry::Directory(dir) = entry else {
                continue;
            };

            if to_string(dir.path.file_name().expect("File name")) != name {
                continue;
            }

            return Some(dir);
        }

        None
    }

    pub fn find(
        &mut self,
        icon_name: &str,
        out: &mut Vec<IconLocation>,
        deep: bool,
        desc: IconDescriptor,
    ) -> u32 {
        let desc = desc.apply(&self.name);

        let mut found = 0;

        if deep {
            for dir in self.dirs() {
                found += dir.find(icon_name, out, deep, desc.clone());
            }
        }

        let path = self.path.clone();
        for file in self.files() {
            if file.contains(icon_name) {
                let path = path.join(file);
                let file_name = to_string(path.file_stem().unwrap());
                // HACK: don't panic on Err varient: my (admittly cursed) artix system shows
                // entries like:
                // - not-allowed
                // - no-drop
                // etcetc. literally who knows what they are.
                let extension = to_string(path.extension().unwrap_or(OsStr::new("no clue")));
                // let extension = to_string(path.extension().unwrap());
                info!("extension: {extension}");
                if extension.to_string() == "no clue" {
                    info!("found case of interest: 'no clue':\nfile_name: {}\npath: {:?}", file_name, path);
                }

                if extension == "xpm" {
                    continue;
                }
                // we only count exact matches
                if file_name == icon_name {
                    found += 1;
                }

                out.push(IconLocation {
                    file_name,
                    path,
                    descriptor: desc.clone(),
                });
            }
        }

        found
    }
}

pub enum CachedDirEntry {
    File(String),
    Directory(Box<CachedDir>),
}

impl CachedDirEntry {
    pub fn visit(path: &Path) -> io::Result<Option<CachedDirEntry>> {
        Ok(if path.is_dir() {
            Some(CachedDirEntry::Directory(Box::new(CachedDir::visit(path)?)))
        } else if path.is_file() {
            let file_name = to_string(path.file_name().unwrap());
            Some(CachedDirEntry::File(file_name))
        } else {
            None
        })
    }
}

pub struct IconLocation {
    pub path: PathBuf,
    pub file_name: String,
    pub descriptor: IconDescriptor,
}

#[derive(Clone, PartialEq, Eq)]
pub struct IconDescriptor {
    pub size: Option<IconSize>,
    // @2x for example
    pub scale: u32,
    pub theme: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Ord, PartialOrd)]
pub struct IconDescOrd {
    size_score: u16,
    scale: u32,
}

impl Display for IconDescriptor {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.size {
            None => write!(f, "unknown"),
            Some(IconSize::Scalable) => write!(f, "scalable"),
            Some(IconSize::Fixed(size)) => write!(f, "{size}"),
        }?;

        if self.scale != 1 {
            write!(f, "@{}x", self.scale)?;
        }

        Ok(())
    }
}
impl IconDescriptor {
    // Higher is better
    pub fn ord(&self, target: u16) -> IconDescOrd {
        let size_score = match self.size {
            None => 0,
            Some(IconSize::Scalable) => u16::MAX - 1,
            Some(IconSize::Fixed(size)) => match size.cmp(&target) {
                Ordering::Equal => u16::MAX,
                Ordering::Greater => {
                    let distance = size - target;
                    u16::MAX - distance
                }
                Ordering::Less => {
                    let distance = target - size;
                    (u16::MAX / 2) - distance
                }
            },
        };

        IconDescOrd {
            size_score,
            scale: self.scale,
        }
    }
    pub fn apply(mut self, dir_name: &str) -> IconDescriptor {
        let mut dir_name = dir_name.to_string();
        if let Some((left_name, scale)) = dir_name.split_once("@") {
            assert!((1..=2).contains(&scale.len()));

            self.scale = u32::from_str(&scale[0..1]).unwrap();
            dir_name = left_name.to_string();
        }

        // Parse size
        if dir_name == "scalable" {
            self.size = Some(IconSize::Scalable);
        }

        // 32x32 format
        if dir_name.chars().filter(|c| c == &'x').count() == 1 {
            if let Some((left, right)) = dir_name.split_once("x") {
                if let (Ok(left), Ok(right)) = (u16::from_str(left), u16::from_str(right)) {
                    self.size = Some(IconSize::Fixed(left.max(right)));
                }
            }
        }

        // Straight number format (32)
        if let Ok(size) = u16::from_str(&dir_name) {
            self.size = Some(IconSize::Fixed(size));
        }

        if self.theme.is_none() {
            self.theme = Some(dir_name);
        }

        self
    }
}
impl Default for IconDescriptor {
    fn default() -> Self {
        IconDescriptor {
            size: None,
            scale: 1,
            theme: None,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum IconSize {
    Scalable,
    Fixed(u16),
}

fn icon_theme_base_paths() -> Vec<PathBuf> {
    let home_icon_dir = home_dir().expect("No $HOME directory").join(".icons");
    let mut data_dirs: Vec<_> = BaseDirectories::new()
        .map(|bd| {
            let mut data_dirs: Vec<_> = bd
                .get_data_dirs()
                .into_iter()
                .flat_map(|p| [p.join("icons"), p.join("pixmaps")])
                .collect();
            let data_home = bd.get_data_home();
            data_dirs.push(data_home.join("icons"));
            data_dirs.push(data_home.join("pixmaps"));
            data_dirs
        })
        .unwrap_or_default();
    data_dirs.push(home_icon_dir);
    for bufg in &data_dirs {
        info!("Found {bufg:?}");
    }
    data_dirs.into_iter().filter(|p| p.exists()).collect()
}

fn to_string(os: &OsStr) -> String {
    os.to_str().unwrap().to_string()
}
