mod finder;

use crate::config::Config;
use crate::icons::finder::IconFinder;
use crate::shortcuts::{Shortcut, ShortcutId};
use eyre::{Context, ContextCompat};
use ico::IconDir;
use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};
use resvg::tiny_skia;
use resvg::tiny_skia::Pixmap;
use resvg::usvg::{Options, Tree};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{create_dir_all, read_to_string, remove_file};
use std::path::{Path, PathBuf};
use tracing::{error, info, warn};

const PREFERRED_ICON_SIZE: u16 = 32;
const PREFERRED_ICON_SIZE_U32: u32 = PREFERRED_ICON_SIZE as u32;

pub struct IconManager {
    icon_image_dir: PathBuf,
    model_path: PathBuf,
    model: IconsModel,

    seen_icons: HashSet<ShortcutId>,

    finder: Option<IconFinder>,
}

impl IconManager {
    pub fn new(dir: &Path) -> eyre::Result<IconManager> {
        let cache_dir = dir.join("icons");
        create_dir_all(&cache_dir).wrap_err("Failed to create icons dir")?;

        let model_path = cache_dir.join("icons.json");
        let model = Config::<IconsModel>::read_file(&model_path).wrap_err("Failed to read icon")?;

        Ok(IconManager {
            icon_image_dir: cache_dir,
            model_path,
            model,
            seen_icons: Default::default(),
            finder: None,
        })
    }

    pub fn read_icon(&self, id: &ShortcutId) -> Option<PathBuf> {
        let model = self.model.values.get(id)?;
        let path = self.icon_path(model.export_id?);
        path.canonicalize().ok()
    }

    pub fn prepare_icon(&mut self, shortcut: &Shortcut) {
        let Some(source) = shortcut.icon.clone() else {
            return;
        };
        self.seen_icons.insert(shortcut.id.clone());

        if let Some(icon) = self.model.values.get(&shortcut.id) {
            if icon.source_location == source {
                // Skip because they are the same
                return;
            }
        }

        info!("Compiling icon {}", shortcut.name);
        let source_path = PathBuf::from(&source);
        let icon_path = if !source_path.is_absolute() {
            let finder = self.finder.get_or_insert_with(IconFinder::new);

            let mut vec = finder.find(&source);
            // To make the icons order stable!
            vec.sort_by_cached_key(|v| v.path.clone());
            vec.sort_by_cached_key(|v| v.descriptor.theme.clone());
            vec.reverse();
            vec.sort_by_cached_key(|v| v.descriptor.ord(PREFERRED_ICON_SIZE));
            vec.reverse();

            if let Some(location) = vec.first() {
                location.path.clone()
            } else {
                source_path
            }
        } else {
            source_path
        };

        let export_id = match self
            .load_icon(&icon_path)
            .wrap_err_with(|| format!("Icon at {icon_path:?}"))
        {
            Ok(icon) => Some(icon),
            Err(error) => {
                error!("failed to load icon for {}", shortcut.name);
                error!("{error:?}");
                None
            }
        };

        self.model.values.insert(
            shortcut.id.clone(),
            IconEntryModel {
                source_location: source,
                export_id,
            },
        );
    }

    fn load_icon(&mut self, icon_path: &Path) -> eyre::Result<usize> {
        let image = Self::render_icon(icon_path).wrap_err("Could not render icon")?;
        if image.width() < PREFERRED_ICON_SIZE_U32 || image.height() < PREFERRED_ICON_SIZE_U32 {
            warn!("Icon {icon_path:?} is smaller than {PREFERRED_ICON_SIZE_U32}x{PREFERRED_ICON_SIZE_U32}", )
        }
        let image = DynamicImage::from(image).resize_to_fill(
            PREFERRED_ICON_SIZE_U32,
            PREFERRED_ICON_SIZE_U32,
            FilterType::Lanczos3,
        );

        let id = self.model.find_free_id();
        let path = self.icon_path(id);
        image.save(&path).wrap_err("Failed to save rendered icon")?;
        Ok(id)
    }

    pub fn save(&mut self) -> eyre::Result<()> {
        self.purge_unseen_icons();

        Config::write_file(&self.model_path, &self.model).wrap_err("Saving config")?;
        Ok(())
    }

    fn purge_unseen_icons(&mut self) {
        for (path, i) in &mut self.model.values {
            if self.seen_icons.contains(path) {
                continue;
            }

            let Some(export_id) = i.export_id.take() else {
                continue;
            };

            let path = self.icon_image_dir.join(format!("{export_id}.png"));
            if let Err(error) = remove_file(&path) {
                eprintln!("Failed to remove file {path:?}, {error}");
            } else {
                println!("Purged {path:?}");
            }
        }
    }

    fn icon_path(&self, id: usize) -> PathBuf {
        self.icon_image_dir.join(format!("{id}.png"))
    }

    fn render_icon(icon: &Path) -> eyre::Result<RgbaImage> {
        let extension = icon.extension().and_then(|v| v.to_str()).unwrap_or("");
        if extension == "svg" {
            let svg_data = read_to_string(icon).wrap_err("Failed to read svg")?;
            let pixmap = Self::render_svg_icon(&svg_data).wrap_err("Failed to render svg")?;

            Ok(RgbaImage::from_raw(pixmap.width(), pixmap.height(), pixmap.take()).unwrap())
        } else if extension == "ico" {
            let file = std::fs::File::open(icon).wrap_err("Failed to read ico")?;
            let icon_dir = IconDir::read(file).wrap_err("Failed to read ico-dir")?;
            let rgba = Self::render_ico_icon(icon_dir).wrap_err("Failed to render ico")?;
            Ok(rgba)
        } else {
            let image = image::open(icon).wrap_err("Could not read image.")?;
            Ok(image.to_rgba8())
        }
    }

    fn render_ico_icon(icon_dir: IconDir) -> eyre::Result<RgbaImage> {
        let (mut closest_entry_i, mut closest_distance) = (0, i64::MAX);
        let entries = icon_dir.entries();
        for (i, entry) in entries.iter().enumerate() {
            let distance = (32 - entry.width().max(entry.height()) as i64).abs();
            if distance < closest_distance {
                closest_distance = distance;
                closest_entry_i = i;
            }
        }
        // Decode the first entry into an image:
        let image = icon_dir.entries()[closest_entry_i]
            .decode()
            .wrap_err("Failed to decode ico")?;
        let rgba = RgbaImage::from_raw(image.width(), image.height(), image.rgba_data().to_vec())
            .wrap_err("Failed to create rgba-image")?;

        Ok(rgba)
    }
    fn render_svg_icon(svg_data: &str) -> eyre::Result<Pixmap> {
        let opt = Options::default();
        let rtree = Tree::from_str(svg_data, &opt).wrap_err("Parsing svg")?;
        let pixmap_size = rtree.size();
        let mut pixmap = Pixmap::new(pixmap_size.width() as u32, pixmap_size.height() as u32)
            .wrap_err_with(|| format!("Allocating pixmap {pixmap_size:?}"))?;
        resvg::render(
            &rtree,
            tiny_skia::Transform::identity(),
            &mut pixmap.as_mut(),
        );
        Ok(pixmap)
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct IconsModel {
    values: HashMap<ShortcutId, IconEntryModel>,
}

#[derive(Serialize, Deserialize)]
pub struct IconEntryModel {
    source_location: String,
    export_id: Option<usize>,
}

impl IconsModel {
    pub fn find_free_id(&self) -> usize {
        let set: HashSet<usize> = self.values.values().flat_map(|v| v.export_id).collect();
        for id in 0..usize::MAX {
            if !set.contains(&id) {
                return id;
            }
        }

        panic!("Could not find a free id");
    }
}
