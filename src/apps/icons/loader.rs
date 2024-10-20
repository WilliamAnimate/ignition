use crate::apps::icons::finder::IconFinder;
use crate::apps::icons::{IconLocation, PREFERRED_ICON_SIZE, PREFERRED_ICON_SIZE_U32};
use crate::apps::AppId;
use crossbeam::channel::{bounded, unbounded, Receiver, RecvError, Sender, TrySendError};
use eyre::{Context, ContextCompat, Report};
use ico::IconDir;
use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};
use resvg::tiny_skia;
use resvg::tiny_skia::Pixmap;
use resvg::usvg::{Options, Tree};
use std::fs::read_to_string;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{spawn, JoinHandle};
use tracing::{error, info, warn};

pub struct LoadIconTaskRequest {
    pub id: AppId,
    pub app_name: String,
    pub icon: String,
    pub location: IconLocation,
}
pub enum LoadIconTaskResponse {
    Success(IconLocation),
    Fail(Report, IconLocation),
}

pub enum IconLoaderStatus {
    Run,
    Finished,
}

pub struct IconLoader {
    receiver: Receiver<LoadIconTaskResponse>,
    sender: Sender<LoadIconTaskRequest>,
    handle: JoinHandle<()>,
    queue: Vec<LoadIconTaskRequest>,
}

impl IconLoader {
    pub fn new() -> IconLoader {
        let (sender_rq, receiver_rq) = bounded::<LoadIconTaskRequest>(16);
        let (sender_rs, receiver_rs) = unbounded::<LoadIconTaskResponse>();

        let handle = spawn(move || {
            let responder = sender_rs;
            let requester = receiver_rq;

            let mut finder = IconFinder::new();
            loop {
                match requester.recv() {
                    Ok(request) => {
                        let source = request.icon;
                        let source_path = PathBuf::from(&source);
                        let icon_path = if !source_path.is_absolute() {
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

                        let response = match Self::load_icon(&request.location, &icon_path)
                            .wrap_err_with(|| format!("Icon at {icon_path:?}"))
                        {
                            Ok(_) => LoadIconTaskResponse::Success(request.location),
                            Err(error) => {
                                //error!("failed to load icon for {}", request.app_name);
                                //error!("{error:?}");
                                LoadIconTaskResponse::Fail(error, request.location)
                            }
                        };

                        responder.send(response).unwrap();
                    }
                    Err(_) => {
                        // Channel has been dropped
                        break;
                    }
                }
            }
        });

        IconLoader {
            receiver: receiver_rs,
            sender: sender_rq,
            handle,
            queue: vec![],
        }
    }

    pub fn enqueue(&mut self, request: LoadIconTaskRequest) {
        self.queue.push(request);
    }

    pub fn tick(&mut self) -> Vec<LoadIconTaskResponse> {
        let mut remaining = 32;
        while let Some(value) = self.queue.pop() {
            if let Err(error) = self.sender.try_send(value) {
                match error {
                    TrySendError::Full(value) => {
                        self.queue.push(value);
                        break;
                    }
                    TrySendError::Disconnected(_) => {
                        panic!("Sender disconnected");
                    }
                }
            }

            remaining -= 1;
            if remaining == 0 {
                break;
            }
        }

        let mut output = Vec::new();
        while let Ok(value) = self.receiver.try_recv() {
            output.push(value);
        }
        output
    }

    pub fn finish(self) -> Vec<LoadIconTaskResponse> {
        drop(self.sender);
        self.handle.join().unwrap();
        
        let mut output: Vec<LoadIconTaskResponse> = self
            .queue
            .into_iter()
            .map(|v| LoadIconTaskResponse::Fail(Report::msg("Cancelled"), v.location))
            .collect();
        while let Ok(value) = self.receiver.recv() {
            output.push(value);
        }
        output
    }

    fn load_icon(output: &IconLocation, icon_path: &Path) -> eyre::Result<()> {
        let image = Self::render_icon(icon_path).wrap_err("Could not render icon")?;
        if image.width() < PREFERRED_ICON_SIZE_U32 || image.height() < PREFERRED_ICON_SIZE_U32 {
            warn!("Icon {icon_path:?} is smaller than {PREFERRED_ICON_SIZE_U32}x{PREFERRED_ICON_SIZE_U32}", )
        }
        let image = DynamicImage::from(image).resize_to_fill(
            PREFERRED_ICON_SIZE_U32,
            PREFERRED_ICON_SIZE_U32,
            FilterType::Lanczos3,
        );

        image
            .save(&output.path)
            .wrap_err("Failed to save rendered icon")?;
        Ok(())
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
