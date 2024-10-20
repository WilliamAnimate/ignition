mod finder;
mod loader;

use crate::apps::icons::finder::IconFinder;
use crate::apps::icons::loader::{IconLoader, LoadIconTaskRequest, LoadIconTaskResponse};
use crate::apps::{App, AppId};
use crate::config::Config;
use crossbeam::channel::{Receiver, Sender};
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
use tracing::{debug, error, info, warn};

const PREFERRED_ICON_SIZE: u16 = 32;
const PREFERRED_ICON_SIZE_U32: u32 = PREFERRED_ICON_SIZE as u32;

/// The AppIconManager is responsible for finding and displaying application icons.
pub struct AppIconManager {
    icon_image_dir: PathBuf,
    model_path: PathBuf,
    model: IconsModel,

    seen_icons: HashSet<AppId>,

    loader: Option<IconLoader>,
    extensions: HashMap<String, usize>,
    
    to_load: usize,
    to_load_finished: usize,
}

impl AppIconManager {
    pub fn new(dir: &Path) -> eyre::Result<AppIconManager> {
        let cache_dir = dir.join("icons");
        create_dir_all(&cache_dir).wrap_err("Failed to create icons dir")?;

        let model_path = cache_dir.join("icons.json");
        let model = Config::<IconsModel>::read_file(&model_path).wrap_err("Failed to read icon")?;

        Ok(AppIconManager {
            icon_image_dir: cache_dir,
            model_path,
            model,
            seen_icons: Default::default(),
            loader: None,
            extensions: Default::default(),
            to_load: 0,
            to_load_finished: 0,
        })
    }

    pub fn read_icon(&self, id: &AppId) -> Option<PathBuf> {
        let model = self.model.values.get(id)?;
        let path = self.icon_path(model.export_id?);
        path.canonicalize().ok()
    }

    pub fn prepare_icon(&mut self, app: &App) {
        let Some(source) = app.icon.clone() else {
            return;
        };
        self.seen_icons.insert(app.id.clone());

        if let Some(icon) = self.model.values.get(&app.id) {
            if icon.source_location == source {
                // Skip because they are the same
                return;
            }
        }

        self.to_load += 1;
        let icon_location = self.new_location(source.clone(), app.id.clone());
        let loader = self.loader.get_or_insert_with(IconLoader::new);

        loader.enqueue(LoadIconTaskRequest {
            id: app.id.clone(),
            app_name: app.name.clone(),
            icon: source,
            location: icon_location,
        });
        //info!("Compiling icon {}", app.name);
        //         let source_path = PathBuf::from(&source);
        //         let icon_path = if !source_path.is_absolute() {
        //             let finder = self.loader.get_or_insert_with(IconFinder::new);
        // 
        //             let mut vec = finder.find(&source);
        //             // To make the icons order stable!
        //             vec.sort_by_cached_key(|v| v.path.clone());
        //             vec.sort_by_cached_key(|v| v.descriptor.theme.clone());
        //             vec.reverse();
        //             vec.sort_by_cached_key(|v| v.descriptor.ord(PREFERRED_ICON_SIZE));
        //             vec.reverse();
        // 
        //             for loc in &vec {
        //                 let x = loc
        //                     .path
        //                     .extension()
        //                     .unwrap_or_default()
        //                     .to_str()
        //                     .unwrap_or_default();
        //                 *self.extensions.entry(x.to_string()).or_default() += 1;
        //             }
        // 
        //             if let Some(location) = vec.first() {
        //                 location.path.clone()
        //             } else {
        //                 source_path
        //             }
        //         } else {
        //             source_path
        //         };
        // 
        //         let export_id = match self
        //             .load_icon(&icon_path)
        //             .wrap_err_with(|| format!("Icon at {icon_path:?}"))
        //         {
        //             Ok(icon) => Some(icon),
        //             Err(error) => {
        //                 error!("failed to load icon for {}", app.name);
        //                 error!("{error:?}");
        //                 None
        //             }
        //         };
        // 
        //         self.model.values.insert(
        //             app.id.clone(),
        //             IconEntryModel {
        //                 source_location: source,
        //                 export_id,
        //             },
        //         );
    }
    
    pub fn to_load(&self) -> usize {
        self.to_load
    }
    pub fn to_load_finished(&self) -> usize {
        self.to_load_finished
    }
    pub fn tick(&mut self) -> bool {
        if let Some(loader) = &mut self.loader {
            let values = loader.tick();
            self.handle_responses(values);
            true
        } else {
            false
        }
    }
    
    pub fn finish(&mut self) -> eyre::Result<()> {
        if let Some(loader) = self.loader.take() {
            let responses = loader.finish();
            self.handle_responses(responses);
        }
        self.save().wrap_err("Saving")?;
        Ok(())
    }
    
    fn handle_responses(&mut self, responses: Vec<LoadIconTaskResponse>) {
        for response in responses {
            self.to_load_finished += 1;
            match response {
                LoadIconTaskResponse::Success(location) => {
                    self.free_location(true, location);
                }
                LoadIconTaskResponse::Fail(error, location) => {
                    error!("Failed to load icon {error:?}");
                    self.free_location(false, location);
                }
            }
        }
    }

    pub fn save(&mut self) -> eyre::Result<()> {
        for (extension, count) in &self.extensions {
            info!(" - {extension}: {count}")
        }
        self.purge_unseen_icons();

        Config::write_file(&self.model_path, &self.model).wrap_err("Saving config")?;
        Ok(())
    }

    pub fn clear_icons(&mut self) {
        let ids: Vec<AppId> = self.model.values.keys().cloned().collect();
        info!("Clearing {} icons", ids.len());
        for id in ids {
            self.remove_icon(&id);
        }
    }

    fn purge_unseen_icons(&mut self) {
        let mut to_remove = Vec::new();
        for id in self.model.values.keys() {
            if self.seen_icons.contains(id) {
                continue;
            }
            to_remove.push(id.clone());
        }

        if to_remove.is_empty() {
            return;
        }

        info!("Removing {} old icons.", to_remove.len());
        for id in to_remove {
            self.remove_icon(&id);
        }
    }

    fn remove_icon(&mut self, app: &AppId) {
        let Some(mut model) = self.model.values.remove(app) else {
            return;
        };

        let Some(export_id) = model.export_id.take() else {
            return;
        };

        let path = self.icon_image_dir.join(format!("{export_id}.png"));
        if let Err(error) = remove_file(&path) {
            error!("Failed to remove file {path:?}, {error}");
        } else {
            info!("Removed icon at {path:?}");
        }
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
    fn icon_path(&self, id: usize) -> PathBuf {
        self.icon_image_dir.join(format!("{id}.png"))
    }

    fn new_location(&mut self, source_location: String, app: AppId, ) -> IconLocation {
        let id = self.model.find_free_id();
        self.model.values.insert(app.clone(), IconEntryModel {
            source_location,
            export_id: Some(id),
        });
        let location = IconLocation {
            app_id: app,
            export_id: id,
            path: self.icon_path(id),
            freed: false,
        };
        location
    }

    fn free_location(&mut self, in_use: bool, mut id: IconLocation) {
        let model = self.model.values.get_mut(&id.app_id).unwrap();
        if !in_use {
            model.export_id = None;
        } else {
            assert_eq!(model.export_id, Some(id.export_id));
        }
        id.freed = true;
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct IconsModel {
    values: HashMap<AppId, IconEntryModel>,
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

pub struct IconLocation {
    app_id: AppId,
    export_id: usize,
    freed: bool,
    path: PathBuf,
}

impl Drop for IconLocation {
    fn drop(&mut self) {
        if !self.freed {
            panic!("IconLocation never freed");
        }
    }
}
