use std::{env, io};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use base64::Engine;
use eyre::{Context, ContextCompat};
use ini::Ini;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use crate::search::SearchScore;

#[derive(Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ShortcutId(pub(crate) String);

pub struct Shortcut {
    pub id: ShortcutId,
    pub path: PathBuf,
    pub name: String,
    pub exec: String,
    pub icon: Option<String>,
    pub comment: Option<String>,
    pub generic_name: Option<String>,
    pub keywords: Option<String>,
    pub penalized: bool,
    pub score: SearchScore,
}

pub struct ShortcutManager {
    pub shortcuts: HashMap<ShortcutId, Shortcut>,
}

impl ShortcutManager {
    pub fn new() -> eyre::Result<ShortcutManager> {
        let mut shortcuts = HashMap::new();
        for appsdir in find_application_dirs()? {
            let files = match get_dir_desktop_files(&appsdir) {
                Ok(v) => v,
                Err(e) => {
                    println!("Could not list {}: {}", &appsdir.to_string_lossy(), e);
                    continue;
                }
            };
            for dtfile in files {
                let path = dtfile.path();
                let info = Ini::load_from_file_opt(
                    &path, ini::ParseOption { enabled_quote: false, enabled_escape: false },
                ).wrap_err("failed to parse ini")?;
                let sec = info.section(Some("Desktop Entry")).wrap_err("No [Desktop Entry] section")?;


                let no_display = sec.get("NoDisplay").unwrap_or("false") == "true";
                if no_display {
                    continue;
                }

                let terminal = sec.get("Terminal").unwrap_or("false") == "true";
                let name = sec.get("Name").wrap_err("No Name key")?;
                let comment = sec.get("Comment");
                let icon = sec.get("Icon");
                let generic_name = sec.get("GenericName");
                let keywords = sec.get("Keywords");
                let Some(exec) = sec.get("Exec") else {
                    continue;
                };
                let categories: Vec<&str> = sec.get("Categories").unwrap_or("").split(";").collect();
                
                // Generate id
                let mut hasher = Sha256::new();
                hasher.update(name.as_bytes());
                hasher.update(comment.unwrap_or("").as_bytes());
                hasher.update(icon.unwrap_or("").as_bytes());
                hasher.update(generic_name.unwrap_or("").as_bytes());
                hasher.update(keywords.unwrap_or("").as_bytes());
                let result = hasher.finalize();
                let id = ShortcutId(base64::engine::general_purpose::STANDARD.encode(result));

                shortcuts.insert(id.clone(), Shortcut {
                    id,
                    path,
                    name: name.to_string(),
                    exec: exec.to_string(),
                    icon: icon.map(|v| v.to_string()),
                    comment: comment.map(|v| v.to_string()),
                    generic_name: generic_name.map(|v| v.to_string()),
                    keywords: keywords.map(|v| v.to_string()),
                    penalized: terminal|| categories.contains(&"Settings"),
                    score: SearchScore::default(),
                });
            }
        }

        Ok(ShortcutManager {
            shortcuts: shortcuts,
        })
    }
}

fn find_application_dirs() -> io::Result<Vec<PathBuf>> {
    let data_home = match env::var_os("XDG_DATA_HOME") {
        Some(val) => {
            PathBuf::from(val)
        }
        None => {
            let home = dirs::home_dir().ok_or(io::Error::new(io::ErrorKind::Other, "Couldn't get home dir"))?;
            home.join(".local/share")
        }
    };
    let extra_data_dirs = match env::var_os("XDG_DATA_DIRS") {
        Some(val) => {
            env::split_paths(&val).map(PathBuf::from).collect()
        }
        None => {
            vec![PathBuf::from("/usr/local/share"),
                 PathBuf::from("/usr/share")]
        }
    };

    let mut res = Vec::new();
    res.push(data_home.join("applications"));
    for dir in extra_data_dirs {
        res.push(dir.join("applications"));
    }
    Ok(res)
}

fn get_dir_desktop_files(path: &Path) -> io::Result<Vec<std::fs::DirEntry>> {
    match path.read_dir() {
        Ok(readdir) => {
            Ok(
                readdir
                    .filter_map(|v| v.ok())
                    .filter(|e| match e.file_type() {
                        Ok(ft) => ft.is_file() | ft.is_symlink(),
                        _ => false
                    })
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".desktop"))
                    .collect::<Vec<_>>()
            )
        }
        Err(e) => {
            if e.kind() == io::ErrorKind::NotFound {
                Ok(Vec::new())
            } else {
                Err(e)
            }
        }
    }
}
