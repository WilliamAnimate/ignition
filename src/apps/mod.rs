pub mod icons;

use base64::Engine;
use eyre::{Context, ContextCompat};
use ini::{Ini, Properties};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{env, io};

/// A sha256 hash of some of the application properties.
#[derive(Clone, Eq, PartialEq, Hash, Ord, Default, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AppId(pub(crate) String);

pub struct App {
    pub id: AppId,
    pub path: PathBuf,
    pub name: String,
    pub exec: String,
    pub icon: Option<String>,
    pub comment: Option<String>,
    pub generic_name: Option<String>,
    pub keywords: Option<String>,
    pub categories: Option<Vec<String>>,
    pub terminal: bool,
}

impl App {
    pub fn parse(path: PathBuf, properties: &Properties) -> eyre::Result<Option<App>> {
        let no_display = properties.get("NoDisplay").unwrap_or("false") == "true";
        if no_display {
            return Ok(None);
        }

        let terminal = properties.get("Terminal").unwrap_or("false") == "true";
        let name = properties.get("Name").wrap_err("No Name key")?;
        let comment = properties.get("Comment");
        let icon = properties.get("Icon");
        let generic_name = properties.get("GenericName");
        let keywords = properties.get("Keywords");
        let Some(exec) = properties.get("Exec") else {
            return Ok(None);
        };
        let categories = properties
            .get("Categories").map(|string| {
            string.split(";").map(|v| v.to_string()).collect::<Vec<String>>()
        });

        // Generate id
        let mut hasher = Sha256::new();
        hasher.update(name.as_bytes());
        hasher.update(comment.unwrap_or("").as_bytes());
        hasher.update(icon.unwrap_or("").as_bytes());
        hasher.update(generic_name.unwrap_or("").as_bytes());
        hasher.update(keywords.unwrap_or("").as_bytes());
        let result = hasher.finalize();
        let id = AppId(base64::engine::general_purpose::STANDARD.encode(result));
        Ok(Some(App {
            id,
            path,
            name: name.to_string(),
            exec: exec.to_string(),
            icon: icon.map(|v| v.to_string()),
            comment: comment.map(|v| v.to_string()),
            generic_name: generic_name.map(|v| v.to_string()),
            keywords: keywords.map(|v| v.to_string()),
            categories,
            terminal,
        }))
    }
}

/// App manager is responsible for finding applications on your system.
pub struct AppManager {
    pub applications: HashMap<AppId, App>,
}

impl AppManager {
    pub fn new() -> eyre::Result<Self> {
        let mut applications = HashMap::new();
        
        for app_dir in find_application_dirs()? {
            let files = match get_dir_desktop_files(&app_dir) {
                Ok(v) => v,
                Err(e) => {
                    println!("Could not list {}: {}", &app_dir.to_string_lossy(), e);
                    continue;
                }
            };
            for app_file in files {
                let path = app_file.path();
                let info = Ini::load_from_file_opt(
                    &path,
                    ini::ParseOption {
                        enabled_quote: false,
                        enabled_escape: false,
                    },
                )
                .wrap_err("failed to parse ini")?;
                let properties = info
                    .section(Some("Desktop Entry"))
                    .wrap_err("No [Desktop Entry] section")?;

                if let Some(app) = App::parse(path, properties)? {
                    applications.insert(app.id.clone(), app);
                }
            }
        }

        Ok(Self { applications })
    }
}

fn find_application_dirs() -> io::Result<Vec<PathBuf>> {
    let data_home = match env::var_os("XDG_DATA_HOME") {
        Some(val) => PathBuf::from(val),
        None => {
            let home = dirs::home_dir().ok_or(io::Error::new(
                io::ErrorKind::Other,
                "Couldn't get home dir",
            ))?;
            home.join(".local/share")
        }
    };
    let extra_data_dirs = match env::var_os("XDG_DATA_DIRS") {
        Some(val) => env::split_paths(&val).map(PathBuf::from).collect(),
        None => {
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
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
        Ok(readdir) => Ok(readdir
            .filter_map(|v| v.ok())
            .filter(|e| match e.file_type() {
                Ok(ft) => ft.is_file() | ft.is_symlink(),
                _ => false,
            })
            .filter(|e| e.file_name().to_string_lossy().ends_with(".desktop"))
            .collect::<Vec<_>>()),
        Err(e) => {
            if e.kind() == io::ErrorKind::NotFound {
                Ok(Vec::new())
            } else {
                Err(e)
            }
        }
    }
}
